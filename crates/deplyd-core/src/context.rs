//! Everything a command needs to know, worked out once.

use std::path::{Path, PathBuf};

use regex::Regex;

use crate::detect::{DEFAULT_IGNORE_JOBS, WorkflowFacts, facts_from_str, token};
use crate::settings::{Override, OverrideLocation, Settings, override_location};
use crate::yaml::YamlError;

/// Folded so `production-api` and `production-web` both answer to `-E production`.
pub const ENVIRONMENT_ALIASES: &[(&str, &[&str])] = &[
    ("production", &["production", "prod", "prd", "live"]),
    ("staging", &["staging", "stage", "stg"]),
    ("acceptance", &["acceptance", "accept", "uat"]),
    ("test", &["test", "qa"]),
    ("development", &["development", "develop", "dev"]),
    ("preview", &["preview", "sandbox"]),
];

/// `cd` as a whole word, so `cd.yml` and `ci-cd.yml` match but `cdn-purge` does not.
pub const DEFAULT_DEPLOY_PATTERN: &str = r"deploy|release|publish|ship|\bcd\b";

#[derive(Debug)]
pub enum ContextError {
    NoWorkflowDirectory(PathBuf),
    NoDeployWorkflows {
        pattern: String,
        looked_at: usize,
    },
    UnreadableWorkflow {
        file: String,
        error: YamlError,
    },
    BadDeployPattern {
        pattern: String,
        reason: String,
    },
    UnreadableOverride {
        path: PathBuf,
        reason: String,
    },
    UnknownEnvironment {
        requested: String,
        detected: Vec<String>,
        from_settings: bool,
    },
    AmbiguousEnvironment {
        requested: String,
        matched: Vec<String>,
    },
}

impl std::fmt::Display for ContextError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ContextError::NoWorkflowDirectory(path) => {
                write!(f, "no .github/workflows found in {}", path.display())
            }
            ContextError::NoDeployWorkflows { pattern, looked_at } => write!(
                f,
                "no deploy workflows found among {looked_at}; searched names for /{pattern}/ \
                 and no job declares an environment"
            ),
            ContextError::UnreadableWorkflow { file, error } => {
                write!(f, "{file}: {error}")
            }
            ContextError::UnreadableOverride { path, reason } => {
                write!(f, "{} could not be read: {reason}", path.display())
            }
            ContextError::BadDeployPattern { pattern, reason } => {
                write!(
                    f,
                    "deployPattern /{pattern}/ is not a valid regex: {reason}"
                )
            }
            ContextError::UnknownEnvironment { requested, .. } => {
                write!(f, "unknown environment '{requested}'")
            }
            ContextError::AmbiguousEnvironment { requested, matched } => write!(
                f,
                "ambiguous environment '{requested}': matches {}",
                matched.join(", ")
            ),
        }
    }
}

impl std::error::Error for ContextError {}

pub struct Context {
    pub repo_root: PathBuf,
    pub author: String,
    pub settings: Settings,
    pub overrides: Option<Override>,
    pub override_location: OverrideLocation,
    /// Every workflow in `.github/workflows`, readable or not.
    pub facts: Vec<WorkflowFacts>,
    /// Unreadable ones, so the report can name them rather than drop them.
    pub unreadable: Vec<(String, YamlError)>,
    pub deploy_pattern: String,
    /// Workflows that deploy.
    pub deploy_workflows: Vec<usize>,
    pub ignore_jobs: Vec<String>,
    pub environments: Vec<String>,
    /// The chosen environment, empty when the repository names none.
    pub environment: String,
    /// The workflows for the chosen environment.
    pub environment_workflows: Vec<usize>,
    /// When choosing an environment narrowed nothing, runs must be matched another
    /// way.
    pub narrowed_by_name: bool,
}

impl Context {
    /// Reads the repository's workflows and works out what deploys.
    pub fn build(
        repo_root: &Path,
        author: String,
        settings: Settings,
    ) -> Result<Self, ContextError> {
        let location = override_location(repo_root);
        let overrides =
            Override::load(&location.path).map_err(|reason| ContextError::UnreadableOverride {
                path: location.path.clone(),
                reason,
            })?;

        let directory = repo_root.join(".github/workflows");
        if !directory.is_dir() {
            return Err(ContextError::NoWorkflowDirectory(repo_root.to_path_buf()));
        }

        let mut facts = Vec::new();
        let mut unreadable = Vec::new();
        let mut files: Vec<PathBuf> = std::fs::read_dir(&directory)
            .map_err(|_| ContextError::NoWorkflowDirectory(repo_root.to_path_buf()))?
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| {
                path.is_file()
                    && path
                        .extension()
                        .is_some_and(|extension| extension == "yml" || extension == "yaml")
            })
            .collect();
        files.sort();

        for path in files {
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            match facts_from_str(&name, &text) {
                Ok(fact) => facts.push(fact),
                // Must not silently vanish: a missing deploy workflow looks exactly
                // like a repo that does not deploy, and the advice differs.
                Err(error) => unreadable.push((name, error)),
            }
        }

        let deploy_pattern = overrides
            .as_ref()
            .and_then(|o| o.deploy_pattern.clone())
            .unwrap_or_else(|| DEFAULT_DEPLOY_PATTERN.to_string());

        let matcher =
            Regex::new(&deploy_pattern).map_err(|error| ContextError::BadDeployPattern {
                pattern: deploy_pattern.clone(),
                reason: error.to_string(),
            })?;

        let mut deploy_workflows: Vec<usize> = facts
            .iter()
            .enumerate()
            .filter(|(_, fact)| matcher.is_match(&format!("{} {}", fact.file, fact.name)))
            .map(|(index, _)| index)
            .collect();

        if deploy_workflows.is_empty() {
            // Nothing matched by name, so fall back to GitHub's own marker for
            // shipping: a job that declares an environment.
            deploy_workflows = facts
                .iter()
                .enumerate()
                .filter(|(_, fact)| !fact.declared_environments.is_empty())
                .map(|(index, _)| index)
                .collect();
        }

        if deploy_workflows.is_empty() {
            // Last resort, reached only when deplyd would otherwise refuse to run.
            deploy_workflows = facts
                .iter()
                .enumerate()
                .filter(|(_, fact)| fact.deploys_by_action)
                .map(|(index, _)| index)
                .collect();
        }

        if deploy_workflows.is_empty() {
            return Err(ContextError::NoDeployWorkflows {
                pattern: deploy_pattern,
                looked_at: facts.len() + unreadable.len(),
            });
        }

        let ignore_jobs = overrides
            .as_ref()
            .filter(|o| !o.ignore_jobs.is_empty())
            .map(|o| o.ignore_jobs.clone())
            .unwrap_or_else(|| {
                DEFAULT_IGNORE_JOBS
                    .iter()
                    .map(|word| (*word).to_string())
                    .collect()
            });

        let mut context = Self {
            repo_root: repo_root.to_path_buf(),
            author,
            settings,
            overrides,
            override_location: location,
            facts,
            unreadable,
            deploy_pattern,
            environment_workflows: deploy_workflows.clone(),
            deploy_workflows,
            ignore_jobs,
            environments: Vec::new(),
            environment: String::new(),
            narrowed_by_name: false,
        };

        context.environments = context.detect_environments();
        Ok(context)
    }

    pub fn deploy_facts(&self) -> impl Iterator<Item = &WorkflowFacts> {
        self.deploy_workflows
            .iter()
            .map(|index| &self.facts[*index])
    }

    pub fn environment_facts(&self) -> impl Iterator<Item = &WorkflowFacts> {
        self.environment_workflows
            .iter()
            .map(|index| &self.facts[*index])
    }

    /// Three sources, merged, all limited to workflows already identified as deploys.
    fn detect_environments(&self) -> Vec<String> {
        if let Some(overrides) = &self.overrides
            && !overrides.environments.is_empty()
        {
            // Names given by hand replace the detected ones entirely.
            return overrides.environments.keys().cloned().collect();
        }

        let mut found: Vec<String> = Vec::new();
        let add = |name: String, found: &mut Vec<String>| {
            if !name.is_empty() && !found.contains(&name) {
                found.push(name);
            }
        };

        // 1. Aliases recognised in the workflow file name or title.
        for (canonical, aliases) in ENVIRONMENT_ALIASES {
            let hit = self
                .deploy_facts()
                .any(|fact| aliases.iter().any(|alias| fact.token.contains(alias)));
            if hit {
                add((*canonical).to_string(), &mut found);
            }
        }

        // 2. What the jobs declare, folded so production-api and production-web both
        //    land on production; anything unrecognised keeps its own name.
        for fact in self.deploy_facts() {
            for declared in &fact.declared_environments {
                add(resolve_alias(declared), &mut found);
            }
        }

        // 3. Options of a workflow_dispatch choice input naming the environment.
        for fact in self.deploy_facts() {
            for option in &fact.input_environments {
                add(option.clone(), &mut found);
            }
        }

        found.sort();
        found
    }

    /// Chooses the environment to report on, resolving a prefix to a full name.
    pub fn select_environment(&mut self, requested: &str) -> Result<(), ContextError> {
        let mut wanted = requested.to_string();
        let mut from_settings = false;

        if wanted.is_empty()
            && let Some(remembered) = self.settings.environment.clone()
        {
            wanted = remembered;
            from_settings = true;
        }
        if wanted.is_empty() {
            if self.environments.iter().any(|name| name == "production") {
                wanted = "production".to_string();
            } else if let Some(first) = self.environments.first() {
                wanted = first.clone();
            }
        }

        if !wanted.is_empty() && !self.environments.is_empty() {
            // Exact first, as with commands: with both "prod" and "production"
            // present, -E prod names one of them rather than being ambiguous.
            let exact: Vec<String> = self
                .environments
                .iter()
                .filter(|name| *name == &wanted)
                .cloned()
                .collect();

            let matched = if exact.is_empty() {
                self.environments
                    .iter()
                    .filter(|name| name.starts_with(&wanted))
                    .cloned()
                    .collect::<Vec<_>>()
            } else {
                exact
            };

            match matched.len() {
                1 => wanted = matched[0].clone(),
                0 => {
                    return Err(ContextError::UnknownEnvironment {
                        requested: wanted,
                        detected: self.environments.clone(),
                        from_settings,
                    });
                }
                _ => {
                    return Err(ContextError::AmbiguousEnvironment {
                        requested: wanted,
                        matched,
                    });
                }
            }
        }

        self.environment = wanted.clone();
        if !wanted.is_empty() {
            let selected = self.workflows_for_environment(&wanted);
            if !selected.is_empty() {
                self.narrowed_by_name = selected.len() < self.deploy_workflows.len();
                self.environment_workflows = selected;
            }
        }
        Ok(())
    }

    /// Which workflows belong to an environment.
    pub fn workflows_for_environment(&self, name: &str) -> Vec<usize> {
        // An explicit list in .deplyd.json always wins.
        if let Some(overrides) = &self.overrides
            && let Some(entry) = overrides.environments.get(name)
            && !entry.workflows.is_empty()
        {
            let explicit: Vec<usize> = self
                .facts
                .iter()
                .enumerate()
                .filter(|(_, fact)| entry.workflows.contains(&fact.file))
                .map(|(index, _)| index)
                .collect();
            if !explicit.is_empty() {
                return explicit;
            }
        }

        let aliases: Vec<String> = ENVIRONMENT_ALIASES
            .iter()
            .find(|(canonical, _)| *canonical == name)
            .map(|(_, list)| list.iter().map(|a| (*a).to_string()).collect())
            .unwrap_or_else(|| vec![name.to_string()]);

        let mut matched: Vec<usize> = self
            .deploy_workflows
            .iter()
            .copied()
            .filter(|index| {
                let fact = &self.facts[*index];
                let by_name = aliases.iter().any(|alias| fact.token.contains(alias));
                let declared = fact
                    .declared_environments
                    .iter()
                    .any(|declared| resolve_alias(declared) == name);
                let by_input = fact.input_environments.iter().any(|option| option == name);
                by_name || declared || by_input
            })
            .collect();

        matched.sort_by_key(|index| self.facts[*index].file.clone());
        matched
    }

    /// The scope set by hand for a label, if there is one.
    pub fn scope_override(&self, label: &str) -> Option<Vec<String>> {
        let overrides = self.overrides.as_ref()?;
        overrides
            .scopes
            .iter()
            .find(|(key, _)| key.to_uppercase() == label.to_uppercase())
            .map(|(_, paths)| paths.clone())
    }

    /// "production " or "", for sentences that read the same either way.
    pub fn environment_phrase(&self) -> String {
        if self.environment.is_empty() {
            String::new()
        } else {
            format!("{} ", self.environment)
        }
    }
}

/// Folds a declared environment name onto its canonical spelling, keeping its own
/// name when it matches nothing known.
pub fn resolve_alias(name: &str) -> String {
    let token = token(name);
    for (canonical, aliases) in ENVIRONMENT_ALIASES {
        if aliases.iter().any(|alias| token.contains(alias)) {
            return (*canonical).to_string();
        }
    }
    name.to_lowercase()
}
