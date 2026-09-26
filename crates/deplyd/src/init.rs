//! `init`: write what detection concluded to a file you can correct.
//!
//! Written beside deplyd, not into the repository. Moving it to the repo root as
//! `.deplyd.json` is a deliberate act, and deplyd reads it from there in preference.

use std::collections::BTreeMap;

use anstream::println;

use deplyd_core::context::Context;
use deplyd_core::repo::Repo;
use deplyd_core::settings::{EnvironmentOverride, Override, write_json};
use deplyd_core::targets::{matched_ignore_words, target_label};

use crate::render;
use crate::term::{CYAN, DIM, YELLOW};

/// What detection concluded, in the shape `.deplyd.json` takes. Built from the
/// current conclusion, which already includes whatever the existing file sets -
/// regenerating from bare detection would throw those corrections away.
pub fn draft(context: &Context) -> Override {
    let mut environments = BTreeMap::new();
    for name in &context.environments {
        let workflows = context
            .workflows_for_environment(name)
            .into_iter()
            .map(|index| context.facts[index].file.clone())
            .collect();
        environments.insert(name.clone(), EnvironmentOverride { workflows });
    }

    // Every target deplyd expects, with the scope it inferred; an empty list means it
    // covers everything. Read from the workflow files rather than from runs, since
    // the repos that need this file are the ones where finding targets went wrong.
    let mut scopes: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for fact in context.deploy_facts() {
        for job in &fact.jobs {
            if !matched_ignore_words(&job.name, &context.ignore_jobs).is_empty() {
                continue;
            }
            let label = target_label(&job.name);
            scopes.entry(label).or_insert_with(|| {
                job.working_directory
                    .clone()
                    .into_iter()
                    .collect::<Vec<String>>()
            });
        }
    }

    Override {
        deploy_pattern: Some(context.deploy_pattern.clone()),
        environments,
        ignore_jobs: context.ignore_jobs.clone(),
        scopes,
    }
}

pub fn run(context: &Context, repo: &Repo, force: bool) {
    let path = &context.override_location.path;
    let exists = path.exists();
    let tracked = exists && context.override_location.shared && repo.is_tracked(path);

    if exists && !force {
        // The three cases are not equally consequential, so they must not read alike.
        if tracked {
            render::stop(
                "There is already a .deplyd.json in this repository, and git is tracking it.",
                &[
                    path.display().to_string(),
                    "Nothing was changed. deplyd reads that file in preference, so rewriting it"
                        .into(),
                    "edits a tracked file and shows up as a modification:".into(),
                    "  deplyd init --force".into(),
                    "Read the result with git diff before committing it.".into(),
                ],
            );
        }
        render::stop(
            "There is already a settings file for this repo.",
            &[
                path.display().to_string(),
                "Nothing was changed. To rewrite it from what deplyd concludes now, which".into(),
                "includes whatever that file already sets:".into(),
                "  deplyd init --force".into(),
            ],
        );
    }

    let draft = draft(context);
    if let Err(error) = write_json(path, &draft) {
        render::stop(&format!("Could not write {}: {error}", path.display()), &[]);
    }

    println!();
    println!("{CYAN}Wrote {}{CYAN:#}", path.display());
    println!();

    if tracked {
        // "Changes nothing" is false here: this is a tracked file that just changed.
        println!("{YELLOW}  git is tracking that file, so it now shows as modified.{YELLOW:#}");
        println!("{YELLOW}  Read it with git diff before committing it.{YELLOW:#}");
        println!();
        println!(
            "{DIM}  It holds what deplyd already concluded, so deplyd behaves as before.{DIM:#}"
        );
    } else if context.override_location.shared {
        println!("{YELLOW}  It is in your working tree, though git is not tracking it.{YELLOW:#}");
        println!();
        println!("{DIM}  It holds what detection found, so it changes nothing on its own.{DIM:#}");
    } else {
        println!("{DIM}  It holds what detection found, so it changes nothing on its own.{DIM:#}");
    }

    println!("{DIM}  Edit the lines that are wrong, then run deplyd config to check.{DIM:#}");
    println!();
    println!(
        "{DIM}  environments  {} found: {}{DIM:#}",
        context.environments.len(),
        context.environments.join(", ")
    );
    println!(
        "{DIM}  scopes        {} target(s), empty means covers everything{DIM:#}",
        draft.scopes.len()
    );
    println!("{DIM}  deployPattern the regex that decided which workflows deploy{DIM:#}");
    println!("{DIM}  ignoreJobs    substrings that keep a job from being a target{DIM:#}");
    println!();

    if !context.override_location.shared {
        // The name is the whole mechanism, and "move it as .deplyd.json" reads as a
        // move with a note attached. Print the command so the rename cannot be missed.
        let shared = context.repo_root.join(".deplyd.json");
        let copy = if cfg!(windows) { "copy" } else { "cp" };

        println!("{DIM}  It lives beside deplyd, so your repository is left alone.{DIM:#}");
        println!();
        println!(
            "{DIM}  To share the fixes, copy it into the repo under the name .deplyd.json,{DIM:#}"
        );
        println!("{DIM}  which is the only name deplyd looks for, and commit it:{DIM:#}");
        println!();
        println!("    {copy} \"{}\" \"{}\"", path.display(), shared.display());
        println!();
    }
}
