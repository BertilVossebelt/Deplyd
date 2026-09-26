//! What a repository deploys, read from `.github/workflows`.

use crate::yaml::{Node, YamlError, parse};

/// Plumbing rather than deploys. `deploy-test-api` is a known false positive, and
/// why `ignoreJobs` exists.
pub const DEFAULT_IGNORE_JOBS: &[&str] = &[
    "merge",
    "environment",
    "notify",
    "trigger",
    "lint",
    "test",
    "summary",
    "setup",
    "prepare",
    "complete",
];

/// Actions that ship something. Matched before the version, so a pinned sha
/// does not hide them.
const DEPLOY_ACTIONS: &[&str] = &[
    "azure/webapps-deploy",
    "azure/functions-action",
    "azure/k8s-deploy",
    "aws-actions/amazon-ecs-deploy-task-definition",
    "aws-actions/aws-cloudformation-github-deploy",
    "google-github-actions/deploy-cloudrun",
    "google-github-actions/deploy-appengine",
    "docker/build-push-action",
    "superfly/flyctl-actions",
    "amondnet/vercel-action",
    "nwtgck/actions-netlify",
    "cloudflare/pages-action",
    "cloudflare/wrangler-action",
    "jamesives/github-pages-deploy-action",
    "peaceiris/actions-gh-pages",
    "actions/deploy-pages",
    "pulumi/actions",
];

/// Commands that ship something. The applying forms only: a plan is not a deploy.
const DEPLOY_COMMANDS: &[&str] = &[
    "kubectl apply",
    "kubectl rollout",
    "helm upgrade",
    "terraform apply",
    "serverless deploy",
    "flyctl deploy",
    "fly deploy",
    "vercel --prod",
    "netlify deploy",
    "npm publish",
    "docker push",
    "aws s3 sync",
    "aws ecs update-service",
    "gcloud run deploy",
    "gcloud app deploy",
    "eb deploy",
    "pulumi up",
];

/// Input names that, when they carry a choice, are naming an environment.
const ENVIRONMENT_INPUT_NAMES: &[&str] = &["environment", "env", "target", "stage"];

/// The words in a name, split on punctuation and camelCase. Comparing against the
/// name with punctuation removed made "deploy-latest" contain "test".
pub fn segments(value: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut current = String::new();

    for character in value.chars() {
        if !character.is_ascii_alphanumeric() {
            if !current.is_empty() {
                words.push(std::mem::take(&mut current));
            }
            continue;
        }
        // deployApi -> deploy, Api
        if character.is_ascii_uppercase()
            && current
                .chars()
                .last()
                .is_some_and(|c| c.is_ascii_lowercase())
        {
            words.push(std::mem::take(&mut current));
        }
        current.push(character.to_ascii_lowercase());
    }
    if !current.is_empty() {
        words.push(current);
    }
    words
}

/// Letters and digits only, lowercased, so `Deploy-API` and `deploy api` match.
pub fn token(value: &str) -> String {
    value
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobFacts {
    /// Where the steps work, when they all agree.
    pub step_working_directories: Vec<String>,
    /// The key under `jobs:`, which the API reports for an unnamed job.
    pub key: String,
    /// `name:` if it has one, otherwise the key.
    pub name: String,
    /// As declared, not folded to an alias.
    pub environment: Option<String>,
    /// For a job that only calls another workflow, that workflow's file name.
    pub calls_workflow: Option<String>,
    /// `defaults.run.working-directory`.
    pub working_directory: Option<String>,
    /// What tells matrix legs apart.
    pub matrix_dimensions: Vec<String>,
    pub step_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowFacts {
    /// The workflow's own `paths:` filter, which says what it covers.
    pub trigger_paths: Vec<String>,
    pub file: String,
    pub name: String,
    /// `file` and `name` tokenised together.
    pub token: String,
    pub jobs: Vec<JobFacts>,
    /// What the jobs declare, in file order.
    pub declared_environments: Vec<String>,
    /// Environments offered by a `workflow_dispatch` choice input.
    pub input_environments: Vec<String>,
    /// Every working directory in the file, for the report.
    pub working_directories: Vec<String>,
    /// A checkout taking its ref from an input, so the run's ref is not the answer.
    pub uses_input_ref: bool,
    /// A job calling another workflow, which checks out where this file cannot see.
    pub calls_workflow: bool,
    /// A step that ships something. A last resort when nothing else said so.
    pub deploys_by_action: bool,
}

impl WorkflowFacts {
    /// Whether the commit has to come from the log rather than the run's own ref.
    pub fn needs_log(&self) -> bool {
        self.uses_input_ref || self.calls_workflow
    }

    /// By key or display name. A called workflow arrives as "Deploy API / deploy-api".
    pub fn job(&self, name: &str) -> Option<&JobFacts> {
        let last = name.rsplit('/').next().unwrap_or(name).trim();
        self.jobs
            .iter()
            .find(|job| job.key == name || job.name == name)
            .or_else(|| {
                self.jobs
                    .iter()
                    .find(|job| job.key == last || job.name == last)
            })
    }
}

pub fn facts_from_str(file_name: &str, text: &str) -> Result<WorkflowFacts, YamlError> {
    let document = parse(text)?;

    let stem = file_name
        .rsplit_once('.')
        .map(|(stem, _)| stem)
        .unwrap_or(file_name);
    let name = document
        .get_str(&["name"])
        .map(str::to_string)
        .unwrap_or_else(|| stem.to_string());

    let jobs = read_jobs(&document);

    let mut declared_environments = Vec::new();
    for job in &jobs {
        if let Some(environment) = &job.environment
            && !declared_environments.contains(environment)
        {
            declared_environments.push(environment.clone());
        }
    }

    let mut working_directories = Vec::new();
    collect_working_directories(&document, &mut working_directories);

    Ok(WorkflowFacts {
        trigger_paths: read_trigger_paths(&document),
        deploys_by_action: uses_a_deploy_step(&document),
        file: file_name.to_string(),
        name: name.clone(),
        token: token(&format!("{stem} {name}")),
        uses_input_ref: uses_input_ref(&document),
        calls_workflow: jobs.iter().any(|job| job.calls_workflow.is_some()),
        input_environments: read_input_environments(&document),
        declared_environments,
        working_directories,
        jobs,
    })
}

fn read_jobs(document: &Node) -> Vec<JobFacts> {
    let Some(jobs) = document.get("jobs") else {
        return Vec::new();
    };

    jobs.entries()
        .iter()
        .map(|(key, job)| {
            let name = job
                .get_str(&["name"])
                .map(str::to_string)
                .unwrap_or_else(|| key.clone());

            // A called workflow is named by a path, possibly with a @ref and possibly
            // in another repository. Only the file name is useful here.
            let calls_workflow = job.get_str(&["uses"]).and_then(|uses| {
                let path = uses.split('@').next().unwrap_or(uses);
                let file = path.rsplit('/').next().unwrap_or(path);
                (file.ends_with(".yml") || file.ends_with(".yaml")).then(|| file.to_string())
            });

            JobFacts {
                step_working_directories: step_working_directories(job),
                key: key.clone(),
                name,
                environment: job
                    .get("environment")
                    .and_then(Node::scalar_or_named)
                    .map(str::to_string),
                calls_workflow,
                working_directory: job
                    .get_str(&["defaults", "run", "working-directory"])
                    .map(str::to_string),
                matrix_dimensions: job
                    .at(&["strategy", "matrix"])
                    .map(|matrix| {
                        matrix
                            .entries()
                            .iter()
                            .map(|(dimension, _)| dimension.clone())
                            .collect()
                    })
                    .unwrap_or_default(),
                step_count: job
                    .get("steps")
                    .map(|steps| steps.items().len())
                    .unwrap_or(0),
            }
        })
        .collect()
}

/// Whether any step ships something.
fn uses_a_deploy_step(document: &Node) -> bool {
    let Some(jobs) = document.get("jobs") else {
        return false;
    };

    for (_, job) in jobs.entries() {
        let Some(steps) = job.get("steps") else {
            continue;
        };
        for step in steps.items() {
            if let Some(uses) = step.get_str(&["uses"]) {
                let name = uses.split('@').next().unwrap_or(uses).to_lowercase();
                if DEPLOY_ACTIONS.contains(&name.as_str()) {
                    return true;
                }
            }
            if let Some(run) = step.get_str(&["run"]) {
                let flattened = run.to_lowercase();
                if DEPLOY_COMMANDS
                    .iter()
                    .any(|command| flattened.contains(command))
                {
                    return true;
                }
            }
        }
    }
    false
}

/// Paths a job's steps agree on, and only when they all agree: two steps in
/// different directories mean the job covers both, and guessing is worse.
fn step_working_directories(job: &Node) -> Vec<String> {
    let Some(steps) = job.get("steps") else {
        return Vec::new();
    };

    let named: Vec<&str> = steps
        .items()
        .iter()
        .filter_map(|step| step.get_str(&["working-directory"]))
        .collect();

    if named.is_empty() {
        return Vec::new();
    }
    let first = named[0];
    if named.iter().all(|directory| *directory == first) {
        vec![first.to_string()]
    } else {
        Vec::new()
    }
}

/// Trigger path filters, reduced to plain prefixes. `services/api/**` says the same
/// thing a working-directory does. Negations and mid-pattern wildcards are skipped
/// rather than guessed at.
fn read_trigger_paths(document: &Node) -> Vec<String> {
    let Some(on) = document.get("on") else {
        return Vec::new();
    };

    let mut found: Vec<String> = Vec::new();
    for trigger in ["push", "pull_request"] {
        let Some(paths) = on.at(&[trigger, "paths"]) else {
            continue;
        };
        for item in paths.items() {
            let Some(pattern) = item.as_str() else {
                continue;
            };
            // "!services/api/docs/**" excludes rather than includes, and a pattern
            // starting with a wildcard covers everything.
            if pattern.starts_with('!') || pattern.starts_with('*') {
                continue;
            }
            let prefix = pattern
                .trim_end_matches('*')
                .trim_end_matches('/')
                .to_string();
            // A pattern with a wildcard in the middle names no single directory.
            if prefix.is_empty() || prefix.contains('*') {
                continue;
            }
            if !found.contains(&prefix) {
                found.push(prefix);
            }
        }
    }
    found
}

/// A `workflow_dispatch` choice input naming the environment. The API does not
/// expose what was chosen at dispatch time, but the options say which names exist.
fn read_input_environments(document: &Node) -> Vec<String> {
    let Some(inputs) = document.at(&["on", "workflow_dispatch", "inputs"]) else {
        return Vec::new();
    };

    let mut found = Vec::new();
    for (name, input) in inputs.entries() {
        if !ENVIRONMENT_INPUT_NAMES.contains(&token(name).as_str()) {
            continue;
        }
        let is_choice = input.get_str(&["type"]) == Some("choice");
        let Some(options) = input.get("options") else {
            continue;
        };
        if !is_choice && options.items().is_empty() {
            continue;
        }
        for option in options.items() {
            if let Some(value) = option.as_str() {
                let lowered = value.to_lowercase();
                if !found.contains(&lowered) {
                    found.push(lowered);
                }
            }
        }
    }
    found
}

/// Every `working-directory` in the document, wherever it sits. Walked rather than
/// read from known paths because a step may set one too, and the workflow-level view
/// is meant to describe the file.
fn collect_working_directories(node: &Node, into: &mut Vec<String>) {
    match node {
        Node::Map(entries) => {
            for (key, value) in entries {
                if key == "working-directory"
                    && let Some(directory) = value.as_str()
                    && !into.iter().any(|existing| existing == directory)
                {
                    into.push(directory.to_string());
                }
                collect_working_directories(value, into);
            }
        }
        Node::Seq(items) => {
            for item in items {
                collect_working_directories(item, into);
            }
        }
        _ => {}
    }
}

/// A checkout step whose `ref` comes from a dispatch input. The run's own ref then
/// says nothing about what was deployed, so the log has to be read.
fn uses_input_ref(document: &Node) -> bool {
    fn mentions_input(value: &str) -> bool {
        let compact: String = value.chars().filter(|c| !c.is_whitespace()).collect();
        compact.contains("${{inputs.") || compact.contains("${{github.event.inputs.")
    }

    let Some(jobs) = document.get("jobs") else {
        return false;
    };

    for (_, job) in jobs.entries() {
        let Some(steps) = job.get("steps") else {
            continue;
        };
        for step in steps.items() {
            if let Some(reference) = step.get_str(&["with", "ref"])
                && mentions_input(reference)
            {
                return true;
            }
        }
    }
    false
}
