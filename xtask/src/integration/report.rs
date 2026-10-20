//! Require both a successful assertion report and a clean engine shutdown.

use super::process::Outcome;
use serde::Serialize;
use serde_json::Value;
use std::path::PathBuf;

const MARKER: &str = "GDCEF_ITEST_RESULT ";

#[derive(Clone, Copy, Serialize)]
pub(super) struct Case {
    pub class: &'static str,
    pub case: &'static str,
}

#[derive(Serialize)]
pub(super) struct Assessment {
    pub passed: bool,
    pub failures: Vec<String>,
    pub report: Option<Value>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct CaseResult {
    #[serde(flatten)]
    pub case: Case,
    #[serde(flatten)]
    pub process: Option<Outcome>,
    #[serde(flatten)]
    pub assessment: Assessment,
    pub browser_events: Vec<Value>,
    pub log: Option<PathBuf>,
}

impl CaseResult {
    pub fn setup_failure(error: String) -> Self {
        Self {
            case: Case {
                class: "harness",
                case: "setup",
            },
            process: None,
            assessment: Assessment {
                passed: false,
                failures: vec![error],
                report: None,
            },
            browser_events: Vec::new(),
            log: None,
        }
    }
}

pub(super) fn assess(outcome: &Outcome, expected: Option<Case>) -> Assessment {
    let mut failures = Vec::new();
    if outcome.exit_code != Some(0) || outcome.signal.is_some() {
        failures.push(format!(
            "Process exit code: {:?}; signal: {:?}",
            outcome.exit_code, outcome.signal
        ));
    }
    if outcome.timed_out {
        failures.push("Outer process deadline exceeded".into());
    }
    if let Some(error) = &outcome.process_error {
        failures.push(error.clone());
    }
    if outcome.output_overflow {
        failures.push("Process exceeded the log size limit".into());
    }
    if outcome.log.lines().any(|line| {
        starts_with_ignore_ascii_case(line, "error:")
            || contains_ignore_ascii_case(line, "script error:")
            || contains_ignore_ascii_case(line, "parse error:")
            || contains_ignore_ascii_case(line, "panicked at")
            || (contains_ignore_ascii_case(line, "thread")
                && contains_ignore_ascii_case(line, "panicked"))
    }) {
        failures
            .push("Godot error, extension loading error, or Rust panic diagnostic in log".into());
    }
    let reports: Vec<_> = outcome
        .log
        .lines()
        .filter_map(|line| line.strip_prefix(MARKER))
        .collect();
    let mut report = None;
    if let Some(expected) = expected {
        if reports.len() != 1 {
            failures.push(format!(
                "Expected exactly one test report, received {}",
                reports.len()
            ));
        } else {
            match serde_json::from_str::<Value>(reports[0]) {
                Ok(value) => report = Some(value),
                Err(_) => failures.push("Malformed test report JSON".into()),
            }
        }
        let valid = report.as_ref().is_some_and(|report| {
            report.get("class").and_then(Value::as_str) == Some(expected.class)
                && report.get("case").and_then(Value::as_str) == Some(expected.case)
                && report.get("passed").and_then(Value::as_bool) == Some(true)
                && report
                    .get("checks")
                    .and_then(Value::as_u64)
                    .is_some_and(|checks| checks > 0)
                && report
                    .get("failures")
                    .and_then(Value::as_array)
                    .is_some_and(Vec::is_empty)
        });
        if !valid {
            failures.push("Missing, mismatched, or failed test report".into());
        }
    } else if !reports.is_empty() {
        failures.push("Test addon ran without explicit opt-in during startup smoke".into());
    }
    Assessment {
        passed: failures.is_empty(),
        failures,
        report,
    }
}

pub(super) fn junit(results: &[CaseResult]) -> serde_json::Result<String> {
    let failures = results
        .iter()
        .filter(|item| !item.assessment.passed)
        .count();
    let mut xml = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<testsuite name=\"Godot CEF integration\" tests=\"{}\" failures=\"{failures}\">\n",
        results.len()
    );
    for result in results {
        let seconds = result
            .process
            .as_ref()
            .map_or(0.0, |item| item.duration_ms as f64 / 1000.0);
        xml.push_str(&format!(
            "  <testcase classname=\"{}\" name=\"{}\" time=\"{seconds}\">",
            escape(result.case.class),
            escape(result.case.case)
        ));
        if !result.assessment.passed {
            let message = result
                .assessment
                .failures
                .first()
                .map_or("Integration failure", String::as_str);
            xml.push_str(&format!(
                "<failure message=\"{}\">{}</failure>",
                escape(message),
                escape(&serde_json::to_string_pretty(result)?)
            ));
        }
        xml.push_str("</testcase>\n");
    }
    xml.push_str("</testsuite>\n");
    Ok(xml)
}

/// ASCII-only case-insensitive `starts_with` that avoids allocating a lowercased
/// copy of every scanned log line.
fn starts_with_ignore_ascii_case(value: &str, prefix: &str) -> bool {
    value
        .as_bytes()
        .get(..prefix.len())
        .is_some_and(|head| head.eq_ignore_ascii_case(prefix.as_bytes()))
}

/// ASCII-only case-insensitive substring search. Multi-byte UTF-8 sequences use
/// bytes >= 0x80, so they can never produce a false ASCII match.
fn contains_ignore_ascii_case(value: &str, needle: &str) -> bool {
    let value = value.as_bytes();
    let needle = needle.as_bytes();
    !needle.is_empty()
        && needle.len() <= value.len()
        && value
            .windows(needle.len())
            .any(|window| window.eq_ignore_ascii_case(needle))
}

fn escape(value: &str) -> String {
    value.chars().filter(|&ch| matches!(ch, '\t' | '\n' | '\r' | '\u{20}'..='\u{d7ff}' | '\u{e000}'..='\u{fffd}' | '\u{10000}'..='\u{10ffff}'))
        .collect::<String>().replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
        .replace('"', "&quot;").replace('\'', "&apos;")
}

#[cfg(test)]
// Assertions intentionally panic; Result propagates serialization errors.
#[allow(clippy::panic_in_result_fn)]
mod tests {
    use super::*;
    use serde_json::json;

    const CASE: Case = Case {
        class: "CefTexture",
        case: "js_ipc",
    };

    fn outcome() -> Outcome {
        Outcome {
            exit_code: Some(0),
            signal: None,
            timed_out: false,
            process_error: None,
            output_overflow: false,
            duration_ms: 1250,
            log: format!(
                "{MARKER}{}\n",
                json!({ "class": CASE.class, "case": CASE.case, "passed": true, "checks": 3, "failures": [] })
            ),
        }
    }

    #[test]
    fn success_requires_matching_assertions_and_clean_exit() {
        assert!(assess(&outcome(), Some(CASE)).passed);
        for patch in [
            Outcome {
                exit_code: Some(7),
                ..outcome()
            },
            Outcome {
                exit_code: None,
                signal: Some(11),
                ..outcome()
            },
            Outcome {
                timed_out: true,
                ..outcome()
            },
            Outcome {
                process_error: Some("spawn failed".into()),
                ..outcome()
            },
            Outcome {
                output_overflow: true,
                ..outcome()
            },
            Outcome {
                log: String::new(),
                ..outcome()
            },
            Outcome {
                log: outcome().log.repeat(2),
                ..outcome()
            },
            Outcome {
                log: format!("{MARKER}invalid\n"),
                ..outcome()
            },
        ] {
            assert!(!assess(&patch, Some(CASE)).passed);
        }
        assert!(
            !assess(
                &outcome(),
                Some(Case {
                    class: "CefTexture2D",
                    ..CASE
                })
            )
            .passed
        );
        assert!(!assess(&outcome(), None).passed);
        for diagnostic in [
            "SCRIPT ERROR: example",
            "ERROR: Failed to load extension",
            "Parse Error: fixture",
            "thread 'main' panicked at foo.rs",
            "thread 'main' panicked",
        ] {
            let result = Outcome {
                log: format!("{}{diagnostic}\n", outcome().log),
                ..outcome()
            };
            assert!(!assess(&result, Some(CASE)).passed, "missed: {diagnostic}");
        }
        for benign in [
            "info: café résumé — naïve",
            "thread name without a failure",
            "loaded module",
        ] {
            let result = Outcome {
                log: format!("{}{benign}\n", outcome().log),
                ..outcome()
            };
            assert!(
                assess(&result, Some(CASE)).passed,
                "false positive: {benign}"
            );
        }
        for patch in [
            json!({"passed": false}),
            json!({"checks": 0}),
            json!({"checks": 1.5}),
            json!({"failures": ["failure"]}),
            json!({"failures": null}),
        ] {
            let mut report = json!({ "class": CASE.class, "case": CASE.case, "passed": true, "checks": 3, "failures": [] });
            if let Some(patch) = patch.as_object() {
                for (key, value) in patch {
                    report[key] = value.clone();
                }
            }
            let result = Outcome {
                log: format!("{MARKER}{report}\n"),
                ..outcome()
            };
            assert!(!assess(&result, Some(CASE)).passed);
        }
    }

    #[test]
    fn junit_keeps_failure_evidence_and_json_retains_schema() -> serde_json::Result<()> {
        let result = CaseResult {
            case: CASE,
            process: Some(outcome()),
            assessment: Assessment {
                passed: false,
                failures: vec!["<error a=\"b\"> & failure\u{1}".into()],
                report: None,
            },
            browser_events: Vec::new(),
            log: Some("godot.log".into()),
        };
        let serialized = serde_json::to_value(&result)?;
        assert_eq!(serialized["exitCode"], 0);
        assert_eq!(serialized["case"], "js_ipc");
        assert_eq!(serialized["log"], "godot.log");
        assert!(serialized.get("browserEvents").is_some());
        let xml = junit(&[result])?;
        assert!(xml.contains("tests=\"1\" failures=\"1\""));
        assert!(xml.contains("time=\"1.25\""));
        assert!(xml.contains("&lt;error a=&quot;b&quot;&gt; &amp; failure"));
        assert!(!xml.contains('\u{1}'));
        Ok(())
    }
}
