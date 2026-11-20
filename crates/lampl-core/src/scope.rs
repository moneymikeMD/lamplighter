// SPDX-License-Identifier: Apache-2.0

//! Which projects a recall searches: the named project plus the global projects shared by every project.

/// The global projects when none are configured.
pub const DEFAULT_GLOBAL_PROJECTS: &[&str] = &["general"];

/// Projects a recall searches: empty (the whole store) without `project`, only `project` when `strict`, else `project` plus `globals`.
pub fn recall_projects(project: Option<&str>, strict: bool, globals: &[String]) -> Vec<String> {
    let Some(p) = project else {
        return Vec::new();
    };
    let mut out = vec![p.to_owned()];
    if !strict {
        for g in globals {
            if !out.contains(g) {
                out.push(g.clone());
            }
        }
    }
    out
}

/// Parses a comma-separated global-project list, dropping blanks; an empty or all-blank value means no global projects.
pub fn parse_global_projects(value: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for p in value.split(',').map(str::trim).filter(|p| !p.is_empty()) {
        if !out.iter().any(|o| o == p) {
            out.push(p.to_owned());
        }
    }
    out
}

/// The global projects from `LAMPL_GLOBAL_PROJECTS`, or `DEFAULT_GLOBAL_PROJECTS` when it is unset.
pub fn global_projects_from_env() -> Vec<String> {
    match std::env::var("LAMPL_GLOBAL_PROJECTS") {
        Ok(v) => parse_global_projects(&v),
        Err(_) => DEFAULT_GLOBAL_PROJECTS
            .iter()
            .map(|s| (*s).to_owned())
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn globals() -> Vec<String> {
        vec!["general".to_owned()]
    }

    #[test]
    fn unscoped_recall_searches_everything() {
        assert!(recall_projects(None, false, &globals()).is_empty());
        assert!(recall_projects(None, true, &globals()).is_empty());
    }

    #[test]
    fn scoped_recall_adds_globals_unless_strict() {
        assert_eq!(
            recall_projects(Some("homelab"), false, &globals()),
            ["homelab", "general"]
        );
        assert_eq!(
            recall_projects(Some("homelab"), true, &globals()),
            ["homelab"]
        );
    }

    #[test]
    fn a_global_project_named_directly_is_not_repeated() {
        assert_eq!(
            recall_projects(Some("general"), false, &globals()),
            ["general"]
        );
    }

    #[test]
    fn global_list_parsing_trims_and_dedupes() {
        assert_eq!(
            parse_global_projects(" general, shared ,,general"),
            ["general", "shared"]
        );
        assert!(parse_global_projects("").is_empty());
        assert!(parse_global_projects(" , ").is_empty());
    }
}
