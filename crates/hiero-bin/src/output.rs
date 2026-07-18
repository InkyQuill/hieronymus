use hiero_core::doctor::{CheckStatus, DoctorReport};

pub fn render_doctor_json(report: &DoctorReport) -> Result<String, serde_json::Error> {
    serde_json::to_string_pretty(report)
}

#[must_use]
pub fn render_doctor_human(report: &DoctorReport) -> String {
    let mut output = String::from("System diagnostics\n");
    for check in &report.checks {
        let status = match check.status {
            CheckStatus::Ok => "OK",
            CheckStatus::Warn => "WARN",
            CheckStatus::Fail => "FAIL",
        };
        output.push_str(&format!("{status} {}: {}\n", check.name, check.detail));
    }
    output
}

#[cfg(test)]
mod tests {
    use hiero_core::doctor::{CheckStatus, DoctorCheck, DoctorReport};

    use super::{render_doctor_human, render_doctor_json};

    fn report() -> DoctorReport {
        DoctorReport {
            checks: vec![DoctorCheck {
                name: "database".into(),
                status: CheckStatus::Warn,
                detail: "database is absent".into(),
            }],
        }
    }

    #[test]
    fn renders_json_as_one_document() {
        let rendered = render_doctor_json(&report()).expect("report should serialize");
        let parsed: serde_json::Value =
            serde_json::from_str(&rendered).expect("renderer should produce valid JSON");
        assert_eq!(parsed["checks"][0]["status"], "warn");
    }

    #[test]
    fn renders_human_status_name_and_detail() {
        assert_eq!(
            render_doctor_human(&report()),
            "System diagnostics\nWARN database: database is absent\n"
        );
    }
}
