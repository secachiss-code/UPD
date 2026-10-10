//! План запуска: allowlist родителя, переменные описания, затем регион.

use std::collections::BTreeMap;
use std::path::PathBuf;

use crate::controller::drop::RunAs;
use crate::identity::plan::allowlisted_env;
use crate::profiles::EnvironmentPreset;

use super::spec::LaunchSpec;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppLaunchPlan {
    pub netns: String,
    pub program: PathBuf,
    pub args: Vec<String>,
    pub cwd: Option<PathBuf>,
    pub env: BTreeMap<String, String>,
    pub run_as: RunAs,
}

pub fn plan(
    spec: &LaunchSpec,
    netns: &str,
    run_as: RunAs,
    preset: &EnvironmentPreset,
    session_env: &BTreeMap<String, String>,
) -> AppLaunchPlan {
    let mut env = allowlisted_env(session_env);
    for (key, value) in &spec.env {
        env.insert(key.clone(), value.clone());
    }
    env.insert("TZ".to_owned(), preset.timezone.clone());
    env.insert("LANG".to_owned(), preset.locale.clone());
    AppLaunchPlan {
        netns: netns.to_owned(),
        program: spec.program.clone(),
        args: spec.args.clone(),
        cwd: spec.cwd.clone(),
        env,
        run_as,
    }
}
