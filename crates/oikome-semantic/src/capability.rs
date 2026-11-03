// Copyright 2025 The Oikome Authors
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! Identity and capability statements (architecture v3 §9):
//!
//! ```sql
//! SHOW IDENTITY
//! ISSUE UCAN ON SPACE <space> [, <space> ...] | * TO '<did>'
//!     [CAN 'space/query' | 'space/read' | 'space/*'] [EXPIRES IN <duration>]
//! ISSUE UCAN ON COMPUTE '<method>' [, ...] | * TO '<did>'
//!     [LIMITS (time = '30s', input = '64MB', output = '64MB', memory = '512MB')]
//!     [EXPIRES IN <duration>]
//! REVOKE UCAN '<token id>'
//! SHOW UCANS
//! GRANT SELECT ON SPACE <space> TO <principal> [, <principal> ...]
//! REVOKE SELECT ON SPACE <space> FROM <principal> [, <principal> ...]
//! ```
//!
//! A whole-Space grant covers every table of the Space, including tables
//! created later, and the views that read only that Space. A principal is a
//! name or a quoted DID, as in `GRANT SELECT ON <table>`.
//!
//! A duration is a number of seconds or a number with a unit: `90s`,
//! `30m`, `12h`, `7d`, quoted or not.

use crate::error::SemanticError;

/// Default lifetime of an issued UCAN.
pub const DEFAULT_UCAN_LIFETIME_SECS: u64 = 24 * 3600;
/// Longest lifetime `ISSUE UCAN` accepts.
pub const MAX_UCAN_LIFETIME_SECS: u64 = 366 * 24 * 3600;
/// Ability granted when `CAN` is omitted.
pub const DEFAULT_ABILITY: &str = "space/query";

/// What `ISSUE UCAN` asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueUcanRequest {
    /// Space names, or `["*"]` for every Space of the node.
    pub spaces: Vec<String>,
    /// DID of the recipient.
    pub audience: String,
    pub ability: String,
    pub lifetime_secs: u64,
}

/// Limits of a compute grant; `None` leaves the default.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ComputeLimits {
    pub time_ms: Option<u64>,
    pub input_bytes: Option<u64>,
    pub output_bytes: Option<u64>,
    pub memory_bytes: Option<u64>,
}

/// What `ISSUE UCAN ON COMPUTE` asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueComputeRequest {
    /// Compute methods (`sql`), or `["*"]` for every method the node offers.
    pub methods: Vec<String>,
    /// DID of the recipient.
    pub audience: String,
    pub limits: ComputeLimits,
    pub lifetime_secs: u64,
}

/// What `GRANT SELECT ON SPACE` or `REVOKE SELECT ON SPACE` asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpaceGrantRequest {
    pub space: String,
    pub principals: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Tok {
    Word(String),
    Quoted(String),
    Comma,
}

impl Tok {
    fn is_kw(&self, kw: &str) -> bool {
        matches!(self, Tok::Word(w) if w.eq_ignore_ascii_case(kw))
    }

    fn value(&self) -> Option<&str> {
        match self {
            Tok::Word(w) | Tok::Quoted(w) => Some(w),
            Tok::Comma => None,
        }
    }
}

fn err(stmt: &str, msg: impl AsRef<str>) -> SemanticError {
    SemanticError::Parse(format!("{stmt}: {}", msg.as_ref()))
}

fn tokenize(input: &str, stmt: &str) -> Result<Vec<Tok>, SemanticError> {
    let mut tokens = Vec::new();
    let mut chars = input.trim().trim_end_matches(';').chars().peekable();
    while let Some(&c) = chars.peek() {
        match c {
            c if c.is_whitespace() => {
                chars.next();
            }
            ',' => {
                chars.next();
                tokens.push(Tok::Comma);
            }
            '\'' | '"' => {
                chars.next();
                let mut value = String::new();
                loop {
                    match chars.next() {
                        Some(q) if q == c => {
                            // A doubled quote is an escaped quote.
                            if chars.peek() == Some(&c) {
                                chars.next();
                                value.push(c);
                            } else {
                                break;
                            }
                        }
                        Some(other) => value.push(other),
                        None => return Err(err(stmt, "unterminated string")),
                    }
                }
                tokens.push(Tok::Quoted(value));
            }
            _ => {
                let mut word = String::new();
                while let Some(&c) = chars.peek() {
                    if c.is_whitespace() || c == ',' || c == '\'' || c == '"' {
                        break;
                    }
                    word.push(c);
                    chars.next();
                }
                tokens.push(Tok::Word(word));
            }
        }
    }
    Ok(tokens)
}

/// Parse a duration: seconds, or a number followed by `s`, `m`, `h` or `d`.
pub fn parse_duration(text: &str) -> Option<u64> {
    let text = text.trim();
    let (digits, unit) = match text.char_indices().find(|(_, c)| !c.is_ascii_digit()) {
        Some((i, _)) => (&text[..i], text[i..].trim()),
        None => (text, "s"),
    };
    let n: u64 = digits.parse().ok()?;
    let scale = match unit.to_ascii_lowercase().as_str() {
        "s" | "sec" | "secs" | "second" | "seconds" => 1,
        "m" | "min" | "mins" | "minute" | "minutes" => 60,
        "h" | "hour" | "hours" => 3600,
        "d" | "day" | "days" => 86_400,
        _ => return None,
    };
    n.checked_mul(scale)
}

fn space_name(tok: &Tok, stmt: &str) -> Result<String, SemanticError> {
    let name = tok
        .value()
        .ok_or_else(|| err(stmt, "expected a space name"))?;
    let ok = name == "*"
        || (!name.is_empty()
            && name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
            && !name.starts_with(|c: char| c.is_ascii_digit()));
    if ok {
        Ok(name.to_string())
    } else {
        Err(err(stmt, format!("invalid space name '{name}'")))
    }
}

/// Parse a size: bytes, or a number followed by `KB`, `MB` or `GB` (powers
/// of 1024).
pub fn parse_size(text: &str) -> Option<u64> {
    let text = text.trim();
    let (digits, unit) = match text.char_indices().find(|(_, c)| !c.is_ascii_digit()) {
        Some((i, _)) => (&text[..i], text[i..].trim()),
        None => (text, "b"),
    };
    let n: u64 = digits.parse().ok()?;
    let scale: u64 = match unit.to_ascii_lowercase().as_str() {
        "b" => 1,
        "kb" | "k" => 1 << 10,
        "mb" | "m" => 1 << 20,
        "gb" | "g" => 1 << 30,
        _ => return None,
    };
    n.checked_mul(scale)
}

/// Whether an `ISSUE UCAN` statement grants compute (`ON COMPUTE`).
pub(crate) fn is_compute_grant(sql: &str) -> bool {
    sql.split_whitespace()
        .nth(3)
        .is_some_and(|w| w.eq_ignore_ascii_case("COMPUTE"))
}

/// `ISSUE UCAN ON COMPUTE ...`
pub(crate) fn parse_issue_compute(sql: &str) -> Result<IssueComputeRequest, SemanticError> {
    const STMT: &str = "ISSUE UCAN ON COMPUTE";
    // Take `LIMITS ( ... )` out first: the tokenizer does not split on
    // parentheses.
    let mut rest = sql.trim().trim_end_matches(';').to_string();
    let mut limits = ComputeLimits::default();
    if let Some(at) = rest.to_ascii_uppercase().find(" LIMITS") {
        let after = &rest[at + " LIMITS".len()..];
        let open = after
            .find('(')
            .filter(|i| after[..*i].trim().is_empty())
            .ok_or_else(|| err(STMT, "expected LIMITS (name = 'value', ...)"))?;
        let close = after
            .find(')')
            .ok_or_else(|| err(STMT, "LIMITS ( without )"))?;
        let inner = after[open + 1..close].to_string();
        for part in inner.split(',').filter(|p| !p.trim().is_empty()) {
            let (key, value) = part.split_once('=').ok_or_else(|| {
                err(
                    STMT,
                    format!("expected name = 'value', found '{}'", part.trim()),
                )
            })?;
            let value = value.trim().trim_matches(|c| c == '\'' || c == '"');
            let size = || {
                parse_size(value)
                    .filter(|v| *v > 0)
                    .ok_or_else(|| err(STMT, format!("invalid size '{value}'")))
            };
            match key.trim().to_ascii_lowercase().as_str() {
                "time" => {
                    let ms = match value.strip_suffix("ms") {
                        Some(ms) => ms.trim().parse().ok(),
                        None => parse_duration(value).and_then(|s| s.checked_mul(1000)),
                    }
                    .filter(|v| *v > 0)
                    .ok_or_else(|| err(STMT, format!("invalid time '{value}'")))?;
                    limits.time_ms = Some(ms);
                }
                "input" => limits.input_bytes = Some(size()?),
                "output" => limits.output_bytes = Some(size()?),
                "memory" => limits.memory_bytes = Some(size()?),
                other => {
                    return Err(err(
                        STMT,
                        format!("unknown limit '{other}' (time, input, output or memory)"),
                    ))
                }
            }
        }
        rest = format!("{}{}", &rest[..at], &after[close + 1..]);
    }
    let toks = tokenize(&rest, STMT)?;
    let mut i = 4; // ISSUE UCAN ON COMPUTE
    let mut methods = Vec::new();
    loop {
        let name = toks
            .get(i)
            .and_then(Tok::value)
            .ok_or_else(|| err(STMT, "expected a compute method"))?;
        let ok = name == "*"
            || (!name.is_empty()
                && name
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_'));
        if !ok {
            return Err(err(STMT, format!("invalid compute method '{name}'")));
        }
        methods.push(name.to_string());
        i += 1;
        if toks.get(i) == Some(&Tok::Comma) {
            i += 1;
        } else {
            break;
        }
    }
    if methods.len() > 1 && methods.iter().any(|m| m == "*") {
        return Err(err(STMT, "'*' cannot be combined with method names"));
    }
    if !toks.get(i).is_some_and(|t| t.is_kw("TO")) {
        return Err(err(STMT, "expected TO"));
    }
    let audience = match toks.get(i + 1) {
        Some(Tok::Quoted(did)) => did.clone(),
        _ => return Err(err(STMT, "expected TO '<did>' (quoted)")),
    };
    i += 2;
    let mut lifetime = None;
    if toks.get(i).is_some_and(|t| t.is_kw("EXPIRES")) {
        if !toks.get(i + 1).is_some_and(|t| t.is_kw("IN")) {
            return Err(err(STMT, "expected EXPIRES IN <duration>"));
        }
        let text = toks
            .get(i + 2)
            .and_then(Tok::value)
            .ok_or_else(|| err(STMT, "expected a duration after EXPIRES IN"))?;
        let secs = parse_duration(text)
            .filter(|s| *s > 0 && *s <= MAX_UCAN_LIFETIME_SECS)
            .ok_or_else(|| err(STMT, format!("invalid duration '{text}'")))?;
        lifetime = Some(secs);
        i += 3;
    }
    if let Some(extra) = toks.get(i) {
        return Err(err(
            STMT,
            format!("unexpected '{}'", extra.value().unwrap_or(",")),
        ));
    }
    Ok(IssueComputeRequest {
        methods,
        audience,
        limits,
        lifetime_secs: lifetime.unwrap_or(DEFAULT_UCAN_LIFETIME_SECS),
    })
}

/// `ISSUE UCAN ...`
pub(crate) fn parse_issue_ucan(sql: &str) -> Result<IssueUcanRequest, SemanticError> {
    const STMT: &str = "ISSUE UCAN";
    let toks = tokenize(sql, STMT)?;
    let mut i = 2; // ISSUE UCAN
    let expect = |kw: &str, i: &mut usize| -> Result<(), SemanticError> {
        if toks.get(*i).is_some_and(|t| t.is_kw(kw)) {
            *i += 1;
            Ok(())
        } else {
            Err(err(STMT, format!("expected {kw}")))
        }
    };
    expect("ON", &mut i)?;
    expect("SPACE", &mut i)?;

    let mut spaces = Vec::new();
    loop {
        let tok = toks
            .get(i)
            .ok_or_else(|| err(STMT, "expected a space name"))?;
        spaces.push(space_name(tok, STMT)?);
        i += 1;
        if toks.get(i) == Some(&Tok::Comma) {
            i += 1;
        } else {
            break;
        }
    }
    if spaces.len() > 1 && spaces.iter().any(|s| s == "*") {
        return Err(err(STMT, "'*' cannot be combined with space names"));
    }

    expect("TO", &mut i)?;
    let audience = match toks.get(i) {
        Some(Tok::Quoted(did)) => did.clone(),
        _ => return Err(err(STMT, "expected TO '<did>' (quoted)")),
    };
    i += 1;

    let mut ability = None;
    let mut lifetime = None;
    while i < toks.len() {
        if toks[i].is_kw("CAN") && ability.is_none() {
            ability = Some(
                toks.get(i + 1)
                    .and_then(Tok::value)
                    .ok_or_else(|| err(STMT, "expected CAN '<ability>'"))?
                    .to_string(),
            );
            i += 2;
        } else if toks[i].is_kw("EXPIRES") && lifetime.is_none() {
            if !toks.get(i + 1).is_some_and(|t| t.is_kw("IN")) {
                return Err(err(STMT, "expected EXPIRES IN <duration>"));
            }
            let text = toks
                .get(i + 2)
                .and_then(Tok::value)
                .ok_or_else(|| err(STMT, "expected a duration after EXPIRES IN"))?;
            // `EXPIRES IN 12 hours`
            let (text, used) = match toks.get(i + 3) {
                Some(Tok::Word(unit))
                    if text.chars().all(|c| c.is_ascii_digit())
                        && parse_duration(&format!("1{unit}")).is_some() =>
                {
                    (format!("{text}{unit}"), 4)
                }
                _ => (text.to_string(), 3),
            };
            let secs = parse_duration(&text)
                .ok_or_else(|| err(STMT, format!("invalid duration '{text}'")))?;
            if secs == 0 || secs > MAX_UCAN_LIFETIME_SECS {
                return Err(SemanticError::InvalidProperty(format!(
                    "UCAN lifetime must be between 1 second and {} days",
                    MAX_UCAN_LIFETIME_SECS / 86_400
                )));
            }
            lifetime = Some(secs);
            i += used;
        } else {
            return Err(err(
                STMT,
                format!("unexpected '{}'", toks[i].value().unwrap_or(",")),
            ));
        }
    }

    Ok(IssueUcanRequest {
        spaces,
        audience,
        ability: ability.unwrap_or_else(|| DEFAULT_ABILITY.to_string()),
        lifetime_secs: lifetime.unwrap_or(DEFAULT_UCAN_LIFETIME_SECS),
    })
}

/// `GRANT SELECT ON SPACE <space> TO ...` (`revoke` false) or
/// `REVOKE SELECT ON SPACE <space> FROM ...` (`revoke` true).
pub(crate) fn parse_space_grant(
    sql: &str,
    revoke: bool,
) -> Result<SpaceGrantRequest, SemanticError> {
    let stmt = if revoke {
        "REVOKE SELECT ON SPACE"
    } else {
        "GRANT SELECT ON SPACE"
    };
    let toks = tokenize(sql, stmt)?;
    // GRANT|REVOKE SELECT ON SPACE <space> TO|FROM ...
    let space = match toks.get(4) {
        Some(tok) => space_name(tok, stmt)?,
        None => return Err(err(stmt, "expected a space name")),
    };
    if space == "*" {
        return Err(err(stmt, "name one space; '*' is not a space"));
    }
    let preposition = if revoke { "FROM" } else { "TO" };
    if !toks.get(5).is_some_and(|t| t.is_kw(preposition)) {
        return Err(err(stmt, format!("expected {preposition}")));
    }
    let mut principals = Vec::new();
    let mut i = 6;
    loop {
        match toks.get(i) {
            Some(tok @ (Tok::Word(_) | Tok::Quoted(_))) => {
                let name = tok.value().unwrap_or_default();
                if name.is_empty() {
                    return Err(err(stmt, "a principal must not be empty"));
                }
                principals.push(name.to_string());
            }
            _ => return Err(err(stmt, "expected a principal")),
        }
        i += 1;
        match toks.get(i) {
            Some(Tok::Comma) => i += 1,
            None => break,
            Some(other) => {
                return Err(err(
                    stmt,
                    format!("unexpected '{}'", other.value().unwrap_or(",")),
                ))
            }
        }
    }
    Ok(SpaceGrantRequest { space, principals })
}

/// `REVOKE UCAN '<id>'`
pub(crate) fn parse_revoke_ucan(sql: &str) -> Result<String, SemanticError> {
    const STMT: &str = "REVOKE UCAN";
    let toks = tokenize(sql, STMT)?;
    match &toks[2..] {
        [tok] => tok
            .value()
            .map(str::to_string)
            .ok_or_else(|| err(STMT, "expected a token id")),
        [] => Err(err(STMT, "expected a token id")),
        _ => Err(err(STMT, "expected a single token id")),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn compute_grants_parse_with_limits() {
        let sql = "ISSUE UCAN ON COMPUTE 'sql' TO 'did:key:z6Mkabc' \
                   LIMITS (time = '30s', input = '64MB', output = 1024, memory = '1GB') EXPIRES IN 7d";
        assert!(is_compute_grant(sql));
        let req = parse_issue_compute(sql).unwrap();
        assert_eq!(req.methods, vec!["sql"]);
        assert_eq!(req.audience, "did:key:z6Mkabc");
        assert_eq!(
            req.limits,
            ComputeLimits {
                time_ms: Some(30_000),
                input_bytes: Some(64 << 20),
                output_bytes: Some(1024),
                memory_bytes: Some(1 << 30),
            }
        );
        assert_eq!(req.lifetime_secs, 7 * 86_400);
        let all = parse_issue_compute("issue ucan on compute * to 'did:key:z'").unwrap();
        assert_eq!(all.methods, vec!["*"]);
        assert_eq!(all.limits, ComputeLimits::default());
        for bad in [
            "ISSUE UCAN ON COMPUTE 'SQL' TO 'did:key:z'",
            "ISSUE UCAN ON COMPUTE sql, * TO 'did:key:z'",
            "ISSUE UCAN ON COMPUTE sql TO 'did:key:z' LIMITS (cpu = 2)",
            "ISSUE UCAN ON COMPUTE sql TO 'did:key:z' LIMITS (time = 'soon')",
            "ISSUE UCAN ON COMPUTE sql TO 'did:key:z' LIMITS (input = '0')",
            "ISSUE UCAN ON COMPUTE sql TO did:key:z",
        ] {
            assert!(parse_issue_compute(bad).is_err(), "{bad}");
        }
        assert!(!is_compute_grant("ISSUE UCAN ON SPACE blog TO 'did:key:z'"));
    }

    use super::*;

    #[test]
    fn space_grants() {
        let req = parse_space_grant(
            "GRANT SELECT ON SPACE blog TO alice, \"did:key:z6Mkabc\";",
            false,
        )
        .unwrap();
        assert_eq!(req.space, "blog");
        assert_eq!(req.principals, vec!["alice", "did:key:z6Mkabc"]);

        let req =
            parse_space_grant("revoke select on space blog from 'did:key:z6Mkabc'", true).unwrap();
        assert_eq!(req.principals, vec!["did:key:z6Mkabc"]);

        for bad in [
            "GRANT SELECT ON SPACE blog",
            "GRANT SELECT ON SPACE blog TO",
            "GRANT SELECT ON SPACE * TO alice",
            "GRANT SELECT ON SPACE ../x TO alice",
            "GRANT SELECT ON SPACE blog FROM alice",
            "GRANT SELECT ON SPACE blog TO alice bob",
            "GRANT SELECT ON SPACE blog TO alice,",
        ] {
            assert!(parse_space_grant(bad, false).is_err(), "{bad}");
        }
        assert!(parse_space_grant("REVOKE SELECT ON SPACE blog TO alice", true).is_err());
    }

    #[test]
    fn durations() {
        assert_eq!(parse_duration("90"), Some(90));
        assert_eq!(parse_duration("90s"), Some(90));
        assert_eq!(parse_duration("30m"), Some(1800));
        assert_eq!(parse_duration("12 hours"), Some(43_200));
        assert_eq!(parse_duration("7d"), Some(604_800));
        assert_eq!(parse_duration("1w"), None);
        assert_eq!(parse_duration("h"), None);
        assert_eq!(parse_duration("-1"), None);
        assert_eq!(parse_duration("99999999999999999999d"), None);
    }

    #[test]
    fn issue_ucan_full_and_defaults() {
        let req = parse_issue_ucan(
            "ISSUE UCAN ON SPACE blog, orders TO 'did:key:z6Mkabc' CAN 'space/read' EXPIRES IN '2h';",
        )
        .unwrap();
        assert_eq!(
            req,
            IssueUcanRequest {
                spaces: vec!["blog".into(), "orders".into()],
                audience: "did:key:z6Mkabc".into(),
                ability: "space/read".into(),
                lifetime_secs: 7200,
            }
        );
        let req = parse_issue_ucan("issue ucan on space * to \"did:key:z6Mkabc\"").unwrap();
        assert_eq!(req.spaces, vec!["*".to_string()]);
        assert_eq!(req.ability, DEFAULT_ABILITY);
        assert_eq!(req.lifetime_secs, DEFAULT_UCAN_LIFETIME_SECS);
        let req = parse_issue_ucan(
            "ISSUE UCAN ON SPACE blog TO 'did:key:z' EXPIRES IN 12 hours CAN 'space/*'",
        )
        .unwrap();
        assert_eq!(req.lifetime_secs, 43_200);
        assert_eq!(req.ability, "space/*");
    }

    #[test]
    fn issue_ucan_errors() {
        for bad in [
            "ISSUE UCAN",
            "ISSUE UCAN ON blog TO 'did'",
            "ISSUE UCAN ON SPACE TO 'did'",
            "ISSUE UCAN ON SPACE blog",
            "ISSUE UCAN ON SPACE blog TO did:key:z",
            "ISSUE UCAN ON SPACE blog, * TO 'did'",
            "ISSUE UCAN ON SPACE a.b TO 'did'",
            "ISSUE UCAN ON SPACE blog TO 'did' EXPIRES 1h",
            "ISSUE UCAN ON SPACE blog TO 'did' EXPIRES IN 'soon'",
            "ISSUE UCAN ON SPACE blog TO 'did' EXPIRES IN 0",
            "ISSUE UCAN ON SPACE blog TO 'did' EXPIRES IN 400d",
            "ISSUE UCAN ON SPACE blog TO 'did' CAN",
            "ISSUE UCAN ON SPACE blog TO 'did' CAN 'a' CAN 'b'",
            "ISSUE UCAN ON SPACE blog TO 'did' WITH x",
            "ISSUE UCAN ON SPACE blog TO 'did",
        ] {
            assert!(parse_issue_ucan(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn revoke_ucan() {
        assert_eq!(
            parse_revoke_ucan("REVOKE UCAN '0123456789abcdef0123456789abcdef'").unwrap(),
            "0123456789abcdef0123456789abcdef"
        );
        assert_eq!(parse_revoke_ucan("revoke ucan abc;").unwrap(), "abc");
        assert!(parse_revoke_ucan("REVOKE UCAN").is_err());
        assert!(parse_revoke_ucan("REVOKE UCAN a b").is_err());
    }
}
