pub(crate) mod generate_uuid;
pub(crate) mod knowledge;

use std::sync::Arc;

use anyhow::{Context, Result, ensure};
use rig::tool::{DynamicTool, Tool, ToolExecutionError, ToolSet};
use serde_json::{Value, json};

pub(crate) const INTENT_ARG_NAME: &str = "intent";
pub(crate) const INTENT_ARG_DESCRIPTION: &str = "Brief explanation of why you're calling this function and how it helps \
     achieve the current goal. Use present participle form (e.g., 'Fetching...', \
     'Calculating...', 'Validating...'). \
     Examples: 'Fetching order history to analyze purchasing patterns', \
     'Validating email format before sending notification'.";

/// Register all internal agent tools here so intent is added at one boundary.
#[derive(Default)]
pub(crate) struct InternalTools(ToolSet);

impl InternalTools {
    pub(crate) fn tool(mut self, tool: impl Tool + 'static) -> Self {
        self.0.add_tool(tool);
        self
    }

    pub(crate) fn dynamic_tool(mut self, tool: DynamicTool) -> Self {
        self.0.add_dynamic_tool(tool);
        self
    }

    pub(crate) fn into_tools(self) -> Result<Vec<DynamicTool>> {
        let tools = Arc::new(self.0);
        tools
            .get_tool_definitions()
            .into_iter()
            .map(|definition| {
                let parameters = add_intent_parameter(definition.parameters)
                    .with_context(|| format!("Invalid schema for tool {}", definition.name))?;
                let name = definition.name.clone();
                let tools = Arc::clone(&tools);
                Ok(DynamicTool::new(
                    definition.name,
                    definition.description,
                    parameters,
                    move |context, mut arguments| {
                        let tools = Arc::clone(&tools);
                        let name = name.clone();
                        Box::pin(async move {
                            let Some(Value::String(intent)) = arguments
                                .as_object_mut()
                                .and_then(|args| args.remove(INTENT_ARG_NAME))
                            else {
                                return Err(ToolExecutionError::invalid_args(
                                    "Tool arguments must include an intent string",
                                ));
                            };
                            eprintln!("[tool-intent] {name}: {intent}");
                            let result = tools.execute(&name, arguments.to_string(), context).await;
                            // Refusals are separate from errors in Rig's result representation.
                            match result.error().or_else(|| result.refusal()) {
                                Some(error) => Err(error.clone()),
                                None => Ok(result.output().clone()),
                            }
                        })
                    },
                ))
            })
            .collect()
    }
}

fn add_intent_parameter(mut parameters: Value) -> Result<Value> {
    ensure!(parameters["type"] == "object", "Expected an object schema");
    let schema = parameters
        .as_object_mut()
        .context("Expected a schema object")?;
    let properties = schema
        .entry("properties")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .context("Expected properties to be an object")?;
    ensure!(
        !properties.contains_key(INTENT_ARG_NAME),
        "Tool already defines the reserved intent argument"
    );
    properties.insert(
        INTENT_ARG_NAME.into(),
        json!({ "type": "string", "description": INTENT_ARG_DESCRIPTION }),
    );
    let required = schema
        .entry("required")
        .or_insert_with(|| json!([]))
        .as_array_mut()
        .context("Expected required to be an array")?;
    ensure!(
        required.iter().all(Value::is_string),
        "Expected required names to be strings"
    );
    if !required.iter().any(|name| name == INTENT_ARG_NAME) {
        required.push(json!(INTENT_ARG_NAME));
    }
    Ok(parameters)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rig::tool::{ToolContext, ToolErrorKind, ToolOutput};
    use serde::Deserialize;

    #[test]
    fn intent_schema_preserves_existing_contract() {
        for schema in [
            json!({ "type": "object", "properties": {}, "additionalProperties": false }),
            json!({
                "type": "object",
                "properties": { "name": { "type": "string", "enum": ["example"] } },
                "required": ["name"],
                "additionalProperties": false
            }),
        ] {
            let mut updated = add_intent_parameter(schema.clone()).unwrap();
            assert_eq!(
                updated["properties"][INTENT_ARG_NAME],
                json!({
                    "type": "string", "description": INTENT_ARG_DESCRIPTION
                })
            );
            assert_eq!(
                updated["required"].as_array_mut().unwrap().pop(),
                Some(json!("intent"))
            );
            updated["properties"]
                .as_object_mut()
                .unwrap()
                .remove(INTENT_ARG_NAME);
            if schema.get("required").is_none() {
                updated.as_object_mut().unwrap().remove("required");
            }
            assert_eq!(updated, schema);
        }
        for invalid in [
            json!({ "type": "array" }),
            json!({ "type": "object", "properties": { "intent": {} } }),
            json!({ "type": "object", "properties": [] }),
            json!({ "type": "object", "required": "name" }),
            json!({ "type": "object", "required": [42] }),
        ] {
            assert!(add_intent_parameter(invalid).is_err());
        }
    }

    struct StrictTool;

    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct StrictArgs {
        value: String,
    }

    impl Tool for StrictTool {
        const NAME: &'static str = "strict";
        type Args = StrictArgs;
        type Output = String;
        type Error = std::convert::Infallible;

        fn description(&self) -> String {
            "Test strict argument forwarding".into()
        }

        fn parameters(&self) -> Value {
            json!({
                "type": "object",
                "properties": { "value": { "type": "string" } },
                "required": ["value"],
                "additionalProperties": false
            })
        }

        async fn call(&self, _: &mut ToolContext, args: StrictArgs) -> Result<String, Self::Error> {
            Ok(args.value)
        }
    }

    #[tokio::test]
    async fn typed_tool_requires_intent_and_receives_only_its_arguments() {
        let tools = ToolSet::from_dynamic_tools(
            InternalTools::default()
                .tool(StrictTool)
                .into_tools()
                .unwrap(),
        );
        let mut context = ToolContext::default();
        for arguments in [
            json!({ "value": "hello" }),
            json!({ "value": "hello", "intent": 42 }),
            json!({ "value": "hello", "intent": null }),
            json!([]),
        ] {
            let result = tools
                .execute("strict", arguments.to_string(), &mut context)
                .await;
            assert!(result.is_error_kind(ToolErrorKind::InvalidArgs));
        }
        let result = tools
            .execute(
                "strict",
                json!({ "value": "hello", "intent": "Validating argument forwarding" }).to_string(),
                &mut context,
            )
            .await;
        assert!(result.is_success());
        assert_eq!(result.output().as_text(), Some("hello"));
    }

    #[tokio::test]
    async fn dynamic_tool_preserves_context_outputs_errors_and_refusals() {
        #[derive(Clone)]
        struct Inbound;
        #[derive(Clone)]
        struct Outbound;

        let tool = DynamicTool::new(
            "dynamic",
            "Test dynamic forwarding",
            json!({ "type": "object", "properties": { "mode": { "type": "string" } }, "required": ["mode"] }),
            |context, args| {
                Box::pin(async move {
                    assert!(context.get::<Inbound>().is_some());
                    assert!(args.get("intent").is_none());
                    context.insert_result(Outbound);
                    match args["mode"].as_str().unwrap() {
                        "error" => Err(ToolExecutionError::invalid_args("original error")
                            .with_code("original")),
                        "refusal" => Err(ToolExecutionError::refused("original refusal")),
                        _ => Ok(ToolOutput::json(json!({ "answer": 42 }))),
                    }
                })
            },
        );
        let tools = ToolSet::from_dynamic_tools(
            InternalTools::default()
                .dynamic_tool(tool)
                .into_tools()
                .unwrap(),
        );
        assert_eq!(
            tools.get_tool_definitions()[0].parameters["required"],
            json!(["mode", "intent"])
        );
        for mode in ["success", "error", "refusal"] {
            let mut context = ToolContext::default();
            context.insert(Inbound);
            let result = tools
                .execute(
                    "dynamic",
                    json!({ "mode": mode, "intent": "Validating dynamic tool forwarding" })
                        .to_string(),
                    &mut context,
                )
                .await;
            assert!(context.result::<Outbound>().is_some());
            match mode {
                "error" => {
                    assert!(result.is_error_kind(ToolErrorKind::InvalidArgs));
                    assert_eq!(result.error().unwrap().message(), "original error");
                    assert_eq!(result.error().unwrap().code(), Some("original"));
                }
                "refusal" => {
                    assert!(result.is_refused());
                    assert_eq!(result.refusal().unwrap().message(), "original refusal");
                }
                _ => {
                    assert!(result.is_success());
                    assert_eq!(result.output().as_json(), Some(&json!({ "answer": 42 })));
                }
            }
        }
    }
}
