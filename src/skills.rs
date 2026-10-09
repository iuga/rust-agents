use std::collections::{BTreeMap, btree_map::Entry};
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, ensure};
use config::{Config, File, FileFormat};
use rig::tool::{Tool, ToolContext};
use serde::Deserialize;
use serde_json::json;

/// In-memory catalog of skills, exposed to agents through the `load_skill` tool.
///
/// Files are read at build time; loading a skill does not reread its source file.
pub struct Skills {
    /// Skills keyed by their exact names, ordered lexicographically for the catalog.
    entries: BTreeMap<String, Skill>,
}

/// Metadata from YAML frontmatter and instructions from a `SKILL.md` body.
#[derive(Deserialize)]
pub struct Skill {
    /// Lookup name from frontmatter; must not be blank when parsed from a file.
    pub name: String,
    /// Summary used to help the agent choose when to load this skill.
    pub description: String,
    /// Text after the closing frontmatter delimiter, preserved without trimming.
    /// Populated by the parser rather than deserialized from YAML; may be empty.
    #[serde(skip)]
    pub content: String,
}

impl Skills {
    /// Discovers and reads skills from the supplied folders in order.
    ///
    /// Each folder may contain its own `SKILL.md` or immediate skill subfolders.
    /// Missing folders and files are skipped; duplicate names keep the first skill.
    ///
    /// # Errors
    ///
    /// Returns an error for other filesystem failures or invalid skill contents.
    pub fn build(folders: &[String]) -> Result<Self> {
        let paths = Self::discover(folders)?;
        let entries = Self::read(&paths)?;
        Ok(Self { entries })
    }

    /// Borrows a previously loaded skill by its exact, case-sensitive name.
    ///
    /// # Errors
    ///
    /// Returns an error if the name is absent from the catalog.
    pub fn load(&self, name: &str) -> Result<&Skill> {
        self.entries
            .get(name)
            .with_context(|| format!("[skills] Unknown skill: {name}"))
    }

    /// Collects candidate skill paths in configured folder order, without deduplication.
    /// Propagates errors from discovering any folder.
    fn discover(folders: &[String]) -> Result<Vec<PathBuf>> {
        let mut paths = Vec::new();
        for folder in folders {
            paths.extend(Self::discover_folder(Path::new(folder))?);
        }
        Ok(paths)
    }

    /// Returns a folder's own `SKILL.md` if present; otherwise returns sorted
    /// `SKILL.md` paths for immediate subdirectories, excluding symlink directories.
    ///
    /// Child candidates need not exist. Discovery is not recursive. A missing
    /// folder is logged and skipped; other access or directory-listing errors propagate.
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

    /// Reads and parses candidates in order, skipping missing files.
    ///
    /// Duplicate names are logged and the first skill is retained. Other read
    /// errors, invalid UTF-8, and parse errors abort loading with the file path.
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

    /// Parses YAML frontmatter delimited by `---` lines at the start of the text.
    ///
    /// Requires nonblank `name` and `description` values without trimming their
    /// stored values. Preserves the remaining body, including its line endings.
    ///
    /// # Errors
    ///
    /// Returns an error for missing delimiters, invalid metadata, or blank required fields.
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

/// Arguments supplied by an agent to the `load_skill` tool.
#[derive(Deserialize)]
pub struct SkillArgs {
    /// Exact name of the skill to retrieve from the catalog.
    name: String,
}

/// Tool-facing error that preserves the underlying skill lookup error chain.
#[derive(Debug, thiserror::Error)]
#[error("Skill loading failed: {0:#}")]
pub struct SkillError(#[from] anyhow::Error);

impl Tool for Skills {
    const NAME: &'static str = "load_skill";
    type Args = SkillArgs;
    type Output = String;
    type Error = SkillError;

    /// Describes when to use the tool and lists skill summaries in name order.
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

    /// Returns the tool's JSON schema, requiring a name from the current catalog
    /// and declaring that additional properties are not allowed.
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

    /// Logs the requested name and returns an owned copy of the skill's body.
    /// The tool context is unused; an unknown name becomes a [`SkillError`].
    async fn call(
        &self,
        _context: &mut ToolContext,
        args: SkillArgs,
    ) -> Result<String, SkillError> {
        println!("[tool-call] Load Skill: {}", args.name);
        Ok(self.load(&args.name)?.content.clone())
    }
}
