use std::collections::{BTreeMap, btree_map::Entry};
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, ensure};
use config::{Config, File, FileFormat};
use rig::tool::{Tool, ToolContext};
use serde::Deserialize;
use serde_json::json;

pub struct Skills {
    entries: BTreeMap<String, Skill>,
}

#[derive(Deserialize)]
pub struct Skill {
    pub name: String,
    pub description: String,
    #[serde(skip)]
    pub content: String,
}

impl Skills {
    pub fn build(folders: &[String]) -> Result<Self> {
        let paths = Self::discover(folders)?;
        let entries = Self::read(&paths)?;
        Ok(Self { entries })
    }

    pub fn load(&self, name: &str) -> Result<&Skill> {
        self.entries
            .get(name)
            .with_context(|| format!("[skills] Unknown skill: {name}"))
    }

    fn discover(folders: &[String]) -> Result<Vec<PathBuf>> {
        let mut paths = Vec::new();
        for folder in folders {
            paths.extend(Self::discover_folder(Path::new(folder))?);
        }
        Ok(paths)
    }

    fn discover_folder(folder: &Path) -> Result<Vec<PathBuf>> {
        let children = match fs::read_dir(folder) {
            Ok(children) => children,
            Err(error) if error.kind() == ErrorKind::NotFound => {
                eprintln!("[skills] Folder does not exist: {}", folder.display());
                return Ok(Vec::new());
            }
            Err(error) => {
                return Err(error).with_context(|| {
                    format!("[skills] Cannot read skill folder {}", folder.display())
                });
            }
        };

        let direct = folder.join("SKILL.md");
        if direct
            .try_exists()
            .with_context(|| format!("[skills] Cannot access {}", direct.display()))?
        {
            return Ok(vec![direct]);
        }

        let mut paths = Vec::new();
        for entry in children {
            let entry = entry.with_context(|| format!("Cannot list {}", folder.display()))?;
            let file_type = entry
                .file_type()
                .with_context(|| format!("Cannot inspect {}", entry.path().display()))?;
            if file_type.is_dir() {
                paths.push(entry.path().join("SKILL.md"));
            }
        }
        paths.sort();
        Ok(paths)
    }

    fn read(paths: &[PathBuf]) -> Result<BTreeMap<String, Skill>> {
        let mut entries = BTreeMap::new();
        for path in paths {
            let text = match fs::read_to_string(path) {
                Ok(text) => text,
                Err(error) if error.kind() == ErrorKind::NotFound => continue,
                Err(error) => {
                    return Err(error)
                        .with_context(|| format!("Cannot read skill {}", path.display()));
                }
            };
            let skill = Self::parse(&text)
                .with_context(|| format!("Cannot load skill {}", path.display()))?;
            match entries.entry(skill.name.clone()) {
                Entry::Vacant(entry) => {
                    entry.insert(skill);
                }
                Entry::Occupied(entry) => {
                    eprintln!(
                        "[skills-error] Duplicate skill {:?} at {}; keeping the first",
                        entry.key(),
                        path.display()
                    );
                }
            }
        }
        Ok(entries)
    }

    fn parse(text: &str) -> Result<Skill> {
        let mut lines = text.split_inclusive('\n');
        ensure!(
            lines.next().map(str::trim_end) == Some("---"),
            "Expected YAML frontmatter starting with ---"
        );

        let mut yaml = String::new();
        loop {
            let line = lines
                .next()
                .context("Missing closing --- for frontmatter")?;
            if line.trim_end() == "---" {
                break;
            }
            yaml.push_str(line);
        }

        let mut skill: Skill = Config::builder()
            .add_source(File::from_str(&yaml, FileFormat::Yaml))
            .build()?
            .try_deserialize()?;
        ensure!(
            !skill.name.trim().is_empty(),
            "Skill name must not be empty"
        );
        ensure!(
            !skill.description.trim().is_empty(),
            "Skill description must not be empty"
        );

        skill.content = lines.collect();
        Ok(skill)
    }
}

#[derive(Deserialize)]
pub struct SkillArgs {
    name: String,
}

#[derive(Debug, thiserror::Error)]
#[error("Skill loading failed: {0:#}")]
pub struct SkillError(#[from] anyhow::Error);

impl Tool for Skills {
    const NAME: &'static str = "load_skill";
    type Args = SkillArgs;
    type Output = String;
    type Error = SkillError;

    fn description(&self) -> String {
        let catalog = self
            .entries
            .values()
            .map(|skill| format!("{}: {}", skill.name, skill.description))
            .collect::<Vec<_>>()
            .join("\n");
        format!(
            "Load specialized instructions before performing a task that matches a skill's description. \
             Follow the returned instructions using your available tools. \
             Available skills for this agent:\n{catalog}"
        )
    }

    fn parameters(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "name": {
                    "type": "string",
                    "description": "The exact name of an available skill.",
                    "enum": self.entries.keys().collect::<Vec<_>>()
                }
            },
            "required": ["name"],
            "additionalProperties": false
        })
    }

    async fn call(
        &self,
        _context: &mut ToolContext,
        args: SkillArgs,
    ) -> Result<String, SkillError> {
        println!("[tool-call] Load Skill: {}", args.name);
        Ok(self.load(&args.name)?.content.clone())
    }
}
