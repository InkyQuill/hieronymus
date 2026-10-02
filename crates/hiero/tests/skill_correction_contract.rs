use hiero::application::correction_parser::{OriginReceipt, ParsedIntent, parse_correction_v1};
use hieronymus::data_root::HieronymusConfig;

#[test]
fn generated_correction_examples_parse_in_every_host_bundle() {
    let root = tempfile::tempdir().unwrap();
    let files = hiero::agent_plugins::render(&HieronymusConfig::new(root.path())).unwrap();
    let mut bundles = 0;
    for (path, skill) in &files {
        if !path.ends_with("skills/hieronymus-remember/resources/correction-input.md") {
            continue;
        }
        bundles += 1;
        let commands: Vec<_> = skill
            .lines()
            .filter_map(|line| line.strip_prefix("| `"))
            .map(|line| line.split_once('`').unwrap().0)
            .collect();
        let expected = [
            (ParsedIntent::Rendering, Some("Звёздный свет")),
            (ParsedIntent::Rendering, Some("Кошка")),
            (ParsedIntent::Rendering, Some("The \"Star\"")),
            (ParsedIntent::Invalidate, None),
            (ParsedIntent::Invalidate, None),
            (ParsedIntent::Invalidate, None),
            (ParsedIntent::Qualify, Some("Это лишь предположение Миры.")),
            (ParsedIntent::Qualify, Some("まだ確認されていない")),
        ];
        assert_eq!(commands.len(), expected.len(), "{}", path.display());
        for (command, (intent, value)) in commands.iter().zip(expected) {
            let parsed = parse_correction_v1(&OriginReceipt {
                text: (*command).to_owned(),
            })
            .unwrap_or_else(|reason| panic!("{command}: {reason:?}"));
            assert_eq!(parsed.intent, intent);
            assert_eq!(parsed.decoded_value.as_deref(), value);
        }
        for unsupported in ["переводи это как X", "please translate this as X"] {
            assert!(skill.contains(unsupported));
            assert!(
                parse_correction_v1(&OriginReceipt {
                    text: unsupported.to_owned(),
                })
                .is_err()
            );
        }
        assert!(skill.contains("unsupported_or_ambiguous_command"));
        assert!(skill.contains("Never translate, rewrite or replay an already delivered prompt"));
        assert!(skill.contains("structured correction input through a separate route"));
    }
    assert_eq!(bundles, 6);
}

#[test]
fn generated_bootstrap_discloses_classification_before_retention() {
    let root = tempfile::tempdir().unwrap();
    let files = hiero::agent_plugins::render(&HieronymusConfig::new(root.path())).unwrap();
    let mut bundles = 0;
    for (path, skill) in &files {
        if !path.ends_with("skills/hieronymus-bootstrap/resources/prompt-capture.md") {
            continue;
        }
        bundles += 1;
        for required in [
            "https://api.typesafe.ai/v1/systemone",
            "entire current user message",
            "skipping storage does not undo external transmission",
            "provider/configuration failure",
            "local RU/EN heuristic",
            "skips without local fallback",
            "neither guarantees exclusion",
            "relevance method and fallback diagnostics",
            "#capture-and-stop-capture",
            "may still send/finish an external request",
            "pause is checked again before retention",
            "Relevance acceptance alone is not parsing or application success",
        ] {
            assert!(skill.contains(required), "{}: {required}", path.display());
        }
        assert!(!skill.contains("technical/status messages skip without requiring binding"));
    }
    assert_eq!(bundles, 6);
}

#[test]
fn workflow_skills_ship_their_linked_resources_and_doctor_in_each_bundle() {
    let root = tempfile::tempdir().unwrap();
    let files = hiero::agent_plugins::render(&HieronymusConfig::new(root.path()))
        .unwrap()
        .into_iter()
        .collect::<std::collections::BTreeMap<_, _>>();
    let mut skills = 0;
    for (path, body) in &files {
        if path.file_name().and_then(|n| n.to_str()) != Some("SKILL.md") {
            continue;
        }
        skills += 1;
        for link in body.split("](").skip(1) {
            let target = link.split_once(')').unwrap().0;
            if target.starts_with("resources/") {
                assert!(
                    files.contains_key(&path.parent().unwrap().join(target)),
                    "{}: {target}",
                    path.display()
                );
            }
        }
        if path.ends_with("hieronymus-doctor/SKILL.md") {
            assert!(body.contains("Do not bulk-close sessions by age"));
            assert!(body.contains("do not restart setup blindly"));
        }
    }
    assert_eq!(skills, 6 * 9);
}
