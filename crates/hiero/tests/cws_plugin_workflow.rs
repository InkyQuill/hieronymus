//! Structural bundle checks only; passing them does not establish model compliance.

use std::collections::BTreeMap;

use hieronymus::data_root::HieronymusConfig;

const TARGETS: [&str; 6] = ["codex", "claude", "gemini", "opencode", "openclaw", "pi"];
const SKILLS: [&str; 8] = [
    "hieronymus-bootstrap",
    "hieronymus-recall",
    "hieronymus-learn",
    "hieronymus-read",
    "hieronymus-remember",
    "hieronymus-translate",
    "hieronymus-review",
    "hieronymus-orchestrate",
];
const OBSOLETE_BOUNDARY: &str = "Current scoped terminology contracts are mandatory.";

#[test]
fn every_generated_target_carries_the_project_agreement_workflow() {
    let root = tempfile::tempdir().unwrap();
    let config = HieronymusConfig::new(root.path());
    let rendered = hiero::agent_plugins::render(&config).unwrap();
    let assets = rendered.into_iter().collect::<BTreeMap<_, _>>();
    let expected_resource = include_str!("../resources/cws-project.md");

    for target in TARGETS {
        let target_root = config.agent_plugins_root().join(target);
        let bootstrap_path = target_root.join("skills/hieronymus-bootstrap/SKILL.md");
        let bootstrap = assets
            .get(&bootstrap_path)
            .unwrap_or_else(|| panic!("missing bootstrap skill for {target}"));
        assert!(
            bootstrap.contains("](resources/cws-project.md)"),
            "bootstrap has no local project resource link for {target}"
        );

        let resource_path =
            target_root.join("skills/hieronymus-bootstrap/resources/cws-project.md");
        assert_eq!(
            assets.get(&resource_path).map(String::as_str),
            Some(expected_resource),
            "wrong project resource for {target}"
        );

        for skill in SKILLS {
            let path = target_root.join(format!("skills/{skill}/SKILL.md"));
            let body = assets
                .get(&path)
                .unwrap_or_else(|| panic!("missing {skill} for {target}"));
            assert!(
                !body.contains(OBSOLETE_BOUNDARY),
                "{skill} retained the unconditional terminology boundary for {target}"
            );
            if skill != "hieronymus-bootstrap" {
                let body = body
                    .rsplit_once("\n\n")
                    .expect("shared agreement trailer")
                    .0;
                assert!(
                    body.contains("hieronymus-bootstrap") && body.contains("project agreement"),
                    "{skill} does not defer project context to hieronymus-bootstrap for {target}"
                );
            }
        }
    }
}
