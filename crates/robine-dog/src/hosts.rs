//! Hosts, as their Agent describes them in its metadata payload
//! (`POST /intake/`): the host tags (`env`, `kube_cluster_name`,
//! `DD_TAGS`…) and the aliases of the host name (a Kubernetes node is also
//! `<node>-<cluster>`).
//!
//! Datadog joins both to whatever a host sends: its logs and metrics carry
//! the host tags, and what arrives under an alias belongs to the host. Here
//! that is done at ingestion.

use crate::store::pg::Pg;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::RwLock;

const KIND: &str = "host";

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Host {
    pub name: String,
    /// `key:value`, sorted.
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub aliases: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub os: Option<String>,
}

impl Host {
    /// Reads the Agent's v5 metadata payload; `None` when it names no host.
    /// The tags are only in some of the payloads: `tags` is then `None`.
    fn from_payload(v: &Value) -> Option<(Host, bool)> {
        let name = v
            .get("internalHostname")
            .and_then(Value::as_str)
            .or_else(|| v.pointer("/meta/hostname").and_then(Value::as_str))
            .filter(|h| !h.is_empty())?
            .to_string();
        let strings = |v: Option<&Value>| -> Vec<String> {
            v.and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(Value::as_str)
                        .filter(|s| !s.is_empty())
                        .map(String::from)
                        .collect()
                })
                .unwrap_or_default()
        };
        // One list per source: `system` (the Agent's own), cloud providers…
        let sources = v.get("host-tags").and_then(Value::as_object);
        let mut tags: Vec<String> = sources
            .map(|m| m.values().flat_map(|l| strings(Some(l))).collect())
            .unwrap_or_default();
        tags.sort();
        tags.dedup();
        let mut aliases = strings(v.pointer("/meta/host_aliases"));
        aliases.retain(|a| *a != name);
        aliases.sort();
        aliases.dedup();
        Some((
            Host {
                name,
                tags,
                aliases,
                agent_version: v
                    .get("agentVersion")
                    .and_then(Value::as_str)
                    .map(String::from),
                os: v.get("os").and_then(Value::as_str).map(String::from),
            },
            sources.is_some(),
        ))
    }
}

#[derive(Default)]
struct Inner {
    hosts: HashMap<String, Host>,
    /// Alias → host name.
    aliases: HashMap<String, String>,
}

impl Inner {
    fn put(&mut self, h: Host) {
        self.aliases.retain(|_, name| *name != h.name);
        for a in &h.aliases {
            self.aliases.insert(a.clone(), h.name.clone());
        }
        self.hosts.insert(h.name.clone(), h);
    }
}

#[derive(Default)]
pub struct Hosts {
    inner: RwLock<Inner>,
}

impl Hosts {
    pub async fn load(pg: &Pg) -> anyhow::Result<Self> {
        let mut inner = Inner::default();
        for d in pg.docs(KIND).await? {
            if let Ok(h) = serde_json::from_value::<Host>(d.body) {
                inner.put(h);
            }
        }
        Ok(Self {
            inner: RwLock::new(inner),
        })
    }

    /// Takes in a metadata payload; returns the host when it changed and has
    /// to be saved.
    pub fn update(&self, payload: &Value) -> Option<Host> {
        let (mut h, has_tags) = Host::from_payload(payload)?;
        let mut g = self.inner.write().ok()?;
        if !has_tags && let Some(old) = g.hosts.get(&h.name) {
            h.tags = old.tags.clone();
        }
        if g.hosts.get(&h.name) == Some(&h) {
            return None;
        }
        g.put(h.clone());
        Some(h)
    }

    /// Every host an Agent described, by name.
    pub fn list(&self) -> Vec<Host> {
        let mut hosts: Vec<Host> = self
            .inner
            .read()
            .map(|g| g.hosts.values().cloned().collect())
            .unwrap_or_default();
        hosts.sort_by(|a, b| a.name.cmp(&b.name));
        hosts
    }

    pub async fn save(pg: &Pg, h: &Host) -> anyhow::Result<()> {
        pg.put_doc(KIND, &h.name, &serde_json::to_value(h)?).await?;
        Ok(())
    }

    /// What Datadog does for anything a host sends: an alias becomes the
    /// host name, and the host tags are added, a tag the data already has a
    /// value for excepted.
    pub fn apply(&self, host: &mut Option<String>, tags: &mut Vec<String>) {
        let Some(name) = host.as_deref() else { return };
        let Ok(g) = self.inner.read() else { return };
        if let Some(real) = g.aliases.get(name) {
            *host = Some(real.clone());
        }
        let Some(h) = host.as_deref().and_then(|n| g.hosts.get(n)) else {
            return;
        };
        let mut added = false;
        for t in &h.tags {
            let key = t.split_once(':').map_or(t.as_str(), |(k, _)| k);
            let has = tags.iter().any(|x| {
                x.strip_prefix(key)
                    .is_some_and(|rest| rest.is_empty() || rest.starts_with(':'))
            });
            if !has {
                tags.push(t.clone());
                added = true;
            }
        }
        if added {
            tags.sort();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn payload() -> Value {
        json!({
            "internalHostname": "silo01",
            "agentVersion": "7.85.4",
            "os": "linux",
            "meta": {"hostname": "silo01", "host_aliases": ["node-1-silo01", "silo01"]},
            "host-tags": {"system": ["env:prod", "kube_cluster_name:silo01"], "other": ["team:me"]}
        })
    }

    #[test]
    fn host_tags_and_aliases_are_applied() {
        let hosts = Hosts::default();
        let h = hosts.update(&payload()).unwrap();
        assert_eq!(h.tags, ["env:prod", "kube_cluster_name:silo01", "team:me"]);
        assert_eq!(h.aliases, ["node-1-silo01"]);
        // Nothing new: nothing to save.
        assert!(hosts.update(&payload()).is_none());

        let (mut host, mut tags) = (Some("node-1-silo01".to_string()), vec!["a:b".to_string()]);
        hosts.apply(&mut host, &mut tags);
        assert_eq!(host.as_deref(), Some("silo01"));
        assert_eq!(
            tags,
            ["a:b", "env:prod", "kube_cluster_name:silo01", "team:me"]
        );

        // The data's own value of a tag wins; `environment` is another key.
        let mut tags = vec!["env:staging".to_string(), "environment:x".to_string()];
        hosts.apply(&mut Some("silo01".into()), &mut tags);
        assert!(!tags.contains(&"env:prod".to_string()));
        assert!(tags.contains(&"team:me".to_string()));

        let mut tags = vec![];
        hosts.apply(&mut Some("unknown".into()), &mut tags);
        hosts.apply(&mut None, &mut tags);
        assert!(tags.is_empty());
    }

    #[test]
    fn a_payload_without_tags_keeps_the_known_ones() {
        let hosts = Hosts::default();
        hosts.update(&payload());
        let mut p = payload();
        p.as_object_mut().unwrap().remove("host-tags");
        p["agentVersion"] = json!("7.86.0");
        let h = hosts.update(&p).unwrap();
        assert_eq!(h.tags.len(), 3);
        assert_eq!(h.agent_version.as_deref(), Some("7.86.0"));
        assert!(hosts.update(&json!({"gohai": "{}"})).is_none());
    }
}
