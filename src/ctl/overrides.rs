//! `humpyard ctl overrides`: list, add and remove runtime overrides (spec: `cli`).
//!
//! This is the one `ctl` command that changes anything on the gateway, so it is kept apart from the read-only
//! ones. It is still one request to an endpoint the gateway already serves (`/v1/overrides`), made with the
//! caller's own key, which needs `manage_overrides = true` to change anything; the gateway does the checking.

use std::fmt::Write as _;
use std::io::Write;

use serde_json::{Value, json};

use super::{Failure, Gateway, parse, scalar, table};

/// What `humpyard ctl overrides` was asked to do.
pub enum OverridesCommand {
    List {
        /// Include overrides that have ended (the last 30 days) instead of only those in force.
        history: bool,
    },
    Add {
        disable_target: Option<String>,
        drain_provider: Option<String>,
        /// `ROUTE=TARGET`.
        pin: Option<String>,
        reason: String,
        /// How long it lasts, like `90s`, `30m`, `2h` or `1d`; the gateway's default when `None`.
        duration: Option<String>,
    },
    Remove {
        id: i64,
    },
}

/// Seconds in a duration like `90s`, `30m`, `2h` or `1d`.
///
/// # Errors
/// [`Failure::Unusable`] when the text is not a whole number followed by one of `s`, `m`, `h` or `d`, or is zero.
pub fn duration_secs(text: &str) -> Result<u64, Failure> {
    let bad = || {
        Failure::Unusable(format!(
            "--for `{text}` is not a duration like 90s, 30m, 2h or 1d"
        ))
    };
    let (digits, unit) = text.split_at(text.len().saturating_sub(1));
    let per = match unit {
        "s" => 1,
        "m" => 60,
        "h" => 3_600,
        "d" => 86_400,
        _ => return Err(bad()),
    };
    let count: u64 = digits.parse().map_err(|_| bad())?;
    match count.checked_mul(per) {
        Some(secs) if secs > 0 => Ok(secs),
        _ => Err(bad()),
    }
}

/// A short duration for people: `45s`, `30m`, `2h`, `1h52m`, `1d`.
#[must_use]
pub fn human(secs: u64) -> String {
    let (d, h, m) = (secs / 86_400, secs % 86_400 / 3_600, secs % 3_600 / 60);
    match (d, h, m) {
        (0, 0, 0) => format!("{secs}s"),
        (0, 0, m) => format!("{m}m"),
        (0, h, 0) => format!("{h}h"),
        (0, h, m) => format!("{h}h{m}m"),
        (d, 0, _) => format!("{d}d"),
        (d, h, _) => format!("{d}d{h}h"),
    }
}

/// The body `POST /v1/overrides` takes, from exactly one of the three kinds. Checked before anything is sent.
fn add_body(command: &OverridesCommand) -> Result<Value, Failure> {
    let OverridesCommand::Add {
        disable_target,
        drain_provider,
        pin,
        reason,
        duration,
    } = command
    else {
        return Err(Failure::Unusable("not an add command".into()));
    };
    let chosen = [disable_target, drain_provider, pin]
        .iter()
        .filter(|c| c.is_some())
        .count();
    if chosen != 1 {
        return Err(Failure::Unusable(
            "give exactly one of --disable-target, --drain-provider or --pin ROUTE=TARGET".into(),
        ));
    }
    let mut body = if let Some(target) = disable_target {
        json!({"kind": "disable_target", "subject": target})
    } else if let Some(provider) = drain_provider {
        json!({"kind": "drain_provider", "subject": provider})
    } else {
        let pin = pin.as_deref().unwrap_or_default();
        let (route, target) = pin
            .split_once('=')
            .filter(|(r, t)| !r.is_empty() && !t.is_empty())
            .ok_or_else(|| Failure::Unusable(format!("--pin `{pin}` is not ROUTE=TARGET")))?;
        json!({"kind": "pin_route", "route": route, "subject": target})
    };
    body["reason"] = json!(reason);
    if let Some(text) = duration {
        body["expires_in_secs"] = json!(duration_secs(text)?);
    }
    Ok(body)
}

fn secs(v: &Value) -> u64 {
    v.as_u64().unwrap_or(0)
}

/// The overrides as a table. History adds the status; an ended override shows no time left.
#[must_use]
pub fn render_list(v: &Value, history: bool) -> String {
    let rows: Vec<Value> = v["overrides"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .map(|mut row| {
            let active = row["status"].as_str().is_none_or(|s| s == "active");
            row["left"] = json!(if active {
                human(secs(&row["expires_in_secs"]))
            } else {
                "-".to_string()
            });
            if row["stale"] == json!(true) {
                row["subject"] = json!(format!("{} (stale)", scalar(&row["subject"])));
            }
            row
        })
        .collect();
    if rows.is_empty() {
        return if history {
            "no overrides in the last 30 days\n".into()
        } else {
            "no overrides in force\n".into()
        };
    }
    let mut columns = vec![
        ("ID", "id"),
        ("KIND", "kind"),
        ("SUBJECT", "subject"),
        ("ROUTE", "route"),
        ("LEFT", "left"),
        ("BY", "created_by"),
        ("REASON", "reason"),
    ];
    if history {
        columns.insert(4, ("STATUS", "status"));
    }
    table(&rows, &columns)
}

fn render_created(v: &Value) -> String {
    let mut out = format!(
        "created override {}: {} {}",
        scalar(&v["id"]),
        scalar(&v["kind"]),
        scalar(&v["subject"])
    );
    if let Some(route) = v["route"].as_str() {
        let _ = write!(out, " on route {route}");
    }
    let _ = writeln!(
        out,
        "; expires in {} (at unix ms {})",
        human(secs(&v["expires_in_secs"])),
        scalar(&v["expires_ms"])
    );
    out
}

pub(super) async fn run(
    gateway: &Gateway,
    command: &OverridesCommand,
    json_output: bool,
    out: &mut dyn Write,
) -> Result<(), Failure> {
    let (body, text) = match command {
        OverridesCommand::List { history } => {
            let query = if *history {
                vec![("history", "true".to_string())]
            } else {
                vec![]
            };
            let body = gateway.get("/v1/overrides", &query).await?;
            let text = render_list(&parse(&body)?, *history);
            (body, text)
        }
        OverridesCommand::Add { .. } => {
            // Validated here, so a missing or malformed piece fails before anything is sent.
            let body = gateway.post("/v1/overrides", &add_body(command)?).await?;
            let text = render_created(&parse(&body)?);
            (body, text)
        }
        OverridesCommand::Remove { id } => {
            let body = gateway.delete(&format!("/v1/overrides/{id}")).await?;
            (body, format!("removed override {id}\n"))
        }
    };
    if json_output {
        let _ = writeln!(out, "{}", body.trim_end());
    } else {
        let _ = out.write_all(text.as_bytes());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations_read_the_four_units() {
        assert_eq!(duration_secs("90s").unwrap(), 90);
        assert_eq!(duration_secs("30m").unwrap(), 1_800);
        assert_eq!(duration_secs("2h").unwrap(), 7_200);
        assert_eq!(duration_secs("1d").unwrap(), 86_400);
    }

    #[test]
    fn a_duration_must_be_a_positive_whole_number_with_a_unit() {
        for bad in [
            "",
            "h",
            "2",
            "0h",
            "-1h",
            "1.5h",
            "2w",
            "two hours",
            "99999999999999999999d",
        ] {
            assert!(duration_secs(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn human_durations_are_short() {
        assert_eq!(human(45), "45s");
        assert_eq!(human(1_800), "30m");
        assert_eq!(human(7_200), "2h");
        assert_eq!(human(6_720), "1h52m");
        assert_eq!(human(86_400), "1d");
        assert_eq!(human(90_000), "1d1h");
        assert_eq!(human(0), "0s");
    }

    fn add(
        disable: Option<&str>,
        drain: Option<&str>,
        pin: Option<&str>,
        duration: Option<&str>,
    ) -> OverridesCommand {
        OverridesCommand::Add {
            disable_target: disable.map(str::to_string),
            drain_provider: drain.map(str::to_string),
            pin: pin.map(str::to_string),
            reason: "slow".into(),
            duration: duration.map(str::to_string),
        }
    }

    #[test]
    fn each_kind_builds_its_body() {
        assert_eq!(
            add_body(&add(Some("deep"), None, None, Some("2h"))).unwrap(),
            json!({"kind": "disable_target", "subject": "deep", "reason": "slow", "expires_in_secs": 7200})
        );
        assert_eq!(
            add_body(&add(None, Some("groq"), None, None)).unwrap(),
            json!({"kind": "drain_provider", "subject": "groq", "reason": "slow"})
        );
        assert_eq!(
            add_body(&add(None, None, Some("auto=fast"), None)).unwrap(),
            json!({"kind": "pin_route", "route": "auto", "subject": "fast", "reason": "slow"})
        );
    }

    #[test]
    fn exactly_one_kind_and_a_well_formed_pin_are_required() {
        assert!(add_body(&add(None, None, None, None)).is_err());
        assert!(add_body(&add(Some("a"), Some("b"), None, None)).is_err());
        for bad in ["auto", "auto=", "=fast", ""] {
            assert!(
                add_body(&add(None, None, Some(bad), None)).is_err(),
                "{bad}"
            );
        }
        assert!(add_body(&add(Some("a"), None, None, Some("soon"))).is_err());
    }

    #[test]
    fn the_list_shows_time_left_and_marks_a_stale_subject() {
        let v = json!({"overrides": [
            {"id": 1, "kind": "disable_target", "subject": "deep", "route": null, "reason": "slow",
             "created_by": "ops", "expires_in_secs": 6720, "stale": false},
            {"id": 2, "kind": "pin_route", "subject": "gone", "route": "auto", "reason": "r",
             "created_by": "ops", "expires_in_secs": 60, "stale": true},
        ]});
        let text = render_list(&v, false);
        assert!(text.contains("1h52m"), "{text}");
        assert!(text.contains("gone (stale)"), "{text}");
        assert!(!text.contains("STATUS"), "{text}");
        assert_eq!(
            render_list(&json!({"overrides": []}), false),
            "no overrides in force\n"
        );
    }

    #[test]
    fn the_history_adds_the_status_and_shows_no_time_left_for_an_ended_one() {
        let v = json!({"overrides": [
            {"id": 3, "kind": "disable_target", "subject": "deep", "route": null, "reason": "r",
             "created_by": "ops", "expires_in_secs": 0, "stale": false, "status": "revoked"},
        ]});
        let text = render_list(&v, true);
        assert!(
            text.contains("STATUS") && text.contains("revoked"),
            "{text}"
        );
        assert!(
            !text.contains("0s"),
            "an ended override has no time left: {text}"
        );
        assert_eq!(
            render_list(&json!({"overrides": []}), true),
            "no overrides in the last 30 days\n"
        );
    }
}
