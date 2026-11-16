//! Log Patterns: like Datadog's "Visualize as Patterns", the latest logs
//! matching the query (up to 10,000) are clustered by message shape. Tokens
//! holding digits are variables from the start; lines of the same length and
//! first word are merged when most tokens agree, the differing ones becoming
//! wildcards (Drain-style).

use super::{ApiResult, Scope, compile, time_clause};
use crate::AppState;
use crate::query::sql::Dataset;
use crate::store::greptime::{i64_at, str_at};
use axum::Json;
use axum::extract::State;
use serde::Serialize;
use std::collections::HashMap;

const SAMPLE: i64 = 10_000;
const SIMILARITY: f64 = 0.5;
const WILDCARD: &str = "*";
const MAX_PATTERNS: usize = 500;

#[derive(Serialize, Debug)]
pub struct Pattern {
    pub pattern: String,
    pub count: i64,
    /// The most frequent status.
    pub status: String,
    pub statuses: HashMap<String, i64>,
    pub services: Vec<(String, i64)>,
    pub sample: String,
    pub sample_id: i64,
    /// A search that finds this pattern's logs (its longest fixed phrases).
    pub query: String,
}

#[derive(Serialize)]
pub struct PatternsResp {
    pub sampled: usize,
    pub patterns: Vec<Pattern>,
}

fn is_variable(token: &str) -> bool {
    token.chars().any(|c| c.is_ascii_digit())
}

fn tokenize(message: &str) -> Vec<String> {
    let first_line = message.lines().next().unwrap_or("");
    let mut tokens: Vec<String> = first_line
        .split_whitespace()
        .take(64)
        .map(|t| {
            if is_variable(t) {
                WILDCARD.to_string()
            } else {
                t.to_string()
            }
        })
        .collect();
    if tokens.is_empty() {
        tokens.push(String::new());
    }
    tokens
}

struct Cluster {
    template: Vec<String>,
    count: i64,
    statuses: HashMap<String, i64>,
    services: HashMap<String, i64>,
    sample: String,
    sample_id: i64,
}

/// Drain's similarity: the share of positions where the template has the
/// line's fixed token (variables match nothing). Identical shapes match.
fn similarity(template: &[String], tokens: &[String]) -> f64 {
    if template == tokens {
        return 1.0;
    }
    let same = template
        .iter()
        .zip(tokens)
        .filter(|(a, b)| a == b && *a != WILDCARD)
        .count();
    same as f64 / template.len().max(1) as f64
}

pub struct Log<'a> {
    pub id: i64,
    pub message: &'a str,
    pub status: &'a str,
    pub service: &'a str,
}

pub fn cluster(logs: &[Log]) -> Vec<Pattern> {
    // (length, first token) → clusters
    let mut groups: HashMap<(usize, String), Vec<Cluster>> = HashMap::new();
    for l in logs {
        let tokens = tokenize(l.message);
        let key = (tokens.len(), tokens[0].clone());
        let list = groups.entry(key).or_default();
        // The most similar cluster; on a tie, the one the line changes least
        // (a line of variables joins `GET * *`, not `GET /cart *`).
        let best = list
            .iter_mut()
            .map(|c| {
                let changes = c
                    .template
                    .iter()
                    .zip(&tokens)
                    .filter(|(a, b)| a != b)
                    .count();
                (similarity(&c.template, &tokens), changes, c)
            })
            .filter(|(s, _, _)| *s >= SIMILARITY)
            .max_by(|a, b| a.0.total_cmp(&b.0).then(b.1.cmp(&a.1)));
        let c = match best {
            Some((_, _, c)) => {
                for (t, n) in c.template.iter_mut().zip(&tokens) {
                    if t != n {
                        *t = WILDCARD.to_string();
                    }
                }
                c
            }
            None => {
                list.push(Cluster {
                    template: tokens,
                    count: 0,
                    statuses: HashMap::new(),
                    services: HashMap::new(),
                    sample: l.message.to_string(),
                    sample_id: l.id,
                });
                list.last_mut().unwrap()
            }
        };
        c.count += 1;
        *c.statuses.entry(l.status.to_string()).or_default() += 1;
        *c.services.entry(l.service.to_string()).or_default() += 1;
    }
    // Clusters that ended up with the same template are one pattern.
    let mut merged: HashMap<Vec<String>, Cluster> = HashMap::new();
    for c in groups.into_values().flatten() {
        match merged.get_mut(&c.template) {
            Some(m) => {
                m.count += c.count;
                for (k, v) in c.statuses {
                    *m.statuses.entry(k).or_default() += v;
                }
                for (k, v) in c.services {
                    *m.services.entry(k).or_default() += v;
                }
            }
            None => {
                merged.insert(c.template.clone(), c);
            }
        }
    }
    let mut out: Vec<Pattern> = merged
        .into_values()
        .map(|c| {
            let mut services: Vec<(String, i64)> = c
                .services
                .into_iter()
                .filter(|(s, _)| !s.is_empty())
                .collect();
            services.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
            services.truncate(5);
            let status = c
                .statuses
                .iter()
                .max_by(|a, b| a.1.cmp(b.1).then(b.0.cmp(a.0)))
                .map(|(s, _)| s.clone())
                .unwrap_or_default();
            Pattern {
                pattern: c.template.join(" "),
                query: phrases_query(&c.template),
                count: c.count,
                status,
                statuses: c.statuses,
                services,
                sample: c.sample,
                sample_id: c.sample_id,
            }
        })
        .collect();
    out.sort_by(|a, b| b.count.cmp(&a.count).then(a.pattern.cmp(&b.pattern)));
    out.truncate(MAX_PATTERNS);
    out
}

/// The longest runs of fixed tokens, as quoted phrases (at most three).
fn phrases_query(template: &[String]) -> String {
    let mut runs: Vec<Vec<&str>> = vec![vec![]];
    for t in template {
        if t == WILDCARD || t.contains('"') || t.contains('\\') {
            runs.push(vec![]);
        } else {
            runs.last_mut().unwrap().push(t);
        }
    }
    let mut runs: Vec<String> = runs
        .into_iter()
        .filter(|r| !r.is_empty())
        .map(|r| r.join(" "))
        .filter(|r| r.len() >= 3)
        .collect();
    runs.sort_by_key(|r| std::cmp::Reverse(r.len()));
    runs.truncate(3);
    runs.iter()
        .map(|r| format!("\"{r}\""))
        .collect::<Vec<_>>()
        .join(" ")
}

pub async fn patterns(State(st): State<AppState>, Json(s): Json<Scope>) -> ApiResult<PatternsResp> {
    let filter = compile(&s.query, Dataset::Logs)?;
    let rows = st
        .store
        .gt
        .query(&format!(
            "SELECT id, message, status, service FROM logs WHERE {} AND {filter} ORDER BY ts DESC LIMIT {SAMPLE}",
            time_clause(Dataset::Logs, s.from, s.to)
        ))
        .await?;
    let owned: Vec<(i64, String, String, String)> = rows
        .iter()
        .map(|r| {
            (
                i64_at(r, 0),
                str_at(r, 1).unwrap_or_default(),
                str_at(r, 2).unwrap_or_default(),
                str_at(r, 3).unwrap_or_default(),
            )
        })
        .collect();
    let logs: Vec<Log> = owned
        .iter()
        .map(|(id, m, st, sv)| Log {
            id: *id,
            message: m,
            status: st,
            service: sv,
        })
        .collect();
    Ok(Json(PatternsResp {
        sampled: logs.len(),
        patterns: cluster(&logs),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn log(m: &str) -> Log<'_> {
        Log {
            id: 1,
            message: m,
            status: "info",
            service: "api",
        }
    }

    #[test]
    fn clusters_by_shape() {
        let msgs = [
            "GET /api/cart 200 in 12ms",
            "GET /api/cart 200 in 48ms",
            "GET /api/orders 201 in 7ms",
            "payment failed for user u-1042: card declined",
            "payment failed for user u-77: card declined",
            "cache warmed",
        ];
        let logs: Vec<Log> = msgs.iter().map(|m| log(m)).collect();
        let p = cluster(&logs);
        let got: Vec<(&str, i64)> = p.iter().map(|p| (p.pattern.as_str(), p.count)).collect();
        assert_eq!(
            got,
            vec![
                ("GET /api/cart * in *", 2),
                ("payment failed for user * card declined", 2),
                ("GET /api/orders * in *", 1),
                ("cache warmed", 1),
            ]
        );
        assert_eq!(p[1].query, "\"payment failed for user\" \"card declined\"");
    }

    #[test]
    fn variable_lines_do_not_erase_fixed_paths() {
        let logs = vec![
            log("GET /api/cart 200 12ms"),
            log("GET /api/cart 200 31ms"),
            log("GET /api/products 200 9ms"),
            log("GET /api/cart?u=7 200 12ms"),
            log("GET /api/cart?u=8 200 14ms"),
        ];
        let p: Vec<(String, i64)> = cluster(&logs)
            .into_iter()
            .map(|p| (p.pattern, p.count))
            .collect();
        assert_eq!(
            p,
            vec![
                ("GET * * *".to_string(), 2),
                ("GET /api/cart * *".to_string(), 2),
                ("GET /api/products * *".to_string(), 1),
            ]
        );
    }

    #[test]
    fn different_shapes_stay_apart() {
        let logs = vec![
            log("user logged in"),
            log("user logged out"),
            log("disk full on /var"),
        ];
        let p = cluster(&logs);
        assert_eq!(p[0].pattern, "user logged *");
        assert_eq!(p[0].count, 2);
        assert_eq!(p.len(), 2);
    }
}
