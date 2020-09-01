//! records.syntixi.io/v1alpha1

use std::ops::{Deref, DerefMut};

use crate::cli::Opts;
use crate::providers::{
    util::{ProviderBackend, ZoneDomainName, RecordBuilder},
    ProviderConfig,
};

use anyhow::{anyhow, Result};
use k8s_openapi::api::core::v1::{Pod, Node};
use futures::{StreamExt, TryStreamExt};
use kube::{
    api::{Api, ListParams, WatchEvent},
    Client,
};
use kube_derive::CustomResource;
use serde::{Serialize, Deserialize};

type Selector = std::collections::HashMap<String, String>;

#[derive(Clone, Serialize, Deserialize, Debug)]
enum ExpressionOperator {
    In,
    NotIn,
    Exists,
    DoesNotExist,
}
#[derive(Clone, Serialize, Deserialize, Debug)]
struct Expression {
    pub key: String,
    operator: ExpressionOperator,
    values: Vec<String>,
}
type Expressions = Vec<Expression>;

impl Expression {
    /// Match values based on requirements outlined
    /// [here](https://kubernetes.io/docs/concepts/overview/working-with-objects/labels).
    /// This should be used in combination with a system for matching Labels, as the combination
    /// of Lables and Expressions *together* define what should be returned.
    pub fn match_value(&self, input: Option<&String>) -> bool {
        match &self.operator {
            In => {
                input
                    .and_then(|x| Some(self.values.contains(x)))
                    .unwrap_or(false)
            },
            NotIn => {
                // must exist, see:
                // https://kubernetes.io/docs/concepts/overview/working-with-objects/labels/#resources-that-support-set-based-requirements
                input
                    .and_then(|x| Some(!self.values.contains(x)))
                    .unwrap_or(false)
            },
            Exists => {
                input.is_some()
            },
            DoesNotExist => {
                input.is_none()
            }
        }
    }
}

pub enum RecordChange<'a> {
    Add(&'a String),
    Remove(&'a String)
}

/// `RecordValueCollector` is a trait representing a function that collects values from a dynamic
/// source (the variant of the enum RecordValueFrom), or watches over a set of values and
/// calls a function with the changes that should be made to the relevant records.
///
/// Kubernetes specifies
/// [here](https://kubernetes.io/docs/concepts/overview/working-with-objects/labels)
/// that every value in matchLabels and matchExpressions should evaluate to true in order for
/// a value to correctly match a selector. This should be taken into consideration when
/// implementing a value acquirer.
#[async_trait::async_trait]
pub trait RecordValueCollector {
    /// Return a default ListParams object. This should be overridden per-instance of
    /// RecordValueCollector if the matchLabels arguments can be passed through ListParams, as
    /// otherwise they will need to be parsed manually after acquiring all matching resources.
    fn get_list_parameters(&self) -> ListParams {
        ListParams::default()
    }

    async fn get_values(&self, opts: &Opts) -> Result<Vec<String>>;

    async fn on_value_change(&self, opts: &Opts, provider_config: &mut ProviderConfig,
                             record_builder: &mut RecordBuilder) -> Result<()>;
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct PodSelector {
    #[serde(rename="matchLabels")]
    match_labels: Option<Selector>,
    #[serde(rename="matchExpressions")]
    match_expressions: Option<Expressions>,
}

#[async_trait::async_trait]
impl RecordValueCollector for PodSelector {
    fn get_list_parameters(&self) -> ListParams {
        let mut list_params = RecordValueCollector::get_list_parameters(self);
        if let Some(match_labels) = &self.match_labels {
            for (label, value) in match_labels {
                list_params = list_params.labels(format!("{}={}", label, value).as_str());
            }
        }
        list_params
    }

    /// Query IP addresses from Nodes that are running Pods. The matchLabels field will be passed
    /// to the Kubernetes server through ListParams, and the matchExpressions field will be run
    /// through the Expression::match_value() function.
    ///
    /// Command line configuration:
    /// - --pod-namespace: Namespace to look for Pods
    async fn get_values(&self, opts: &Opts) -> Result<Vec<String>> {
        let list_params = self.get_list_parameters();

        let pods: Api<Pod> = Api::namespaced(Client::try_default().await?,
                                             opts.pod_namespace.as_str());
        let nodes: Api<Node> = Api::all(Client::try_default().await?);

        let pod_list = pods.list(&list_params).await?;

        let mut ips: Vec<String> = Vec::with_capacity(pod_list.items.len());
        let mut node_names: Vec<String> = Vec::with_capacity(pod_list.items.len());

        'outer: for pod in pods.list(&list_params).await? {
            let pod_labels = pod
                .metadata
                .labels
                .ok_or(anyhow!("Unable to get pod.metadata.lables"))?;
            if let Some(match_expressions) = &self.match_expressions {
                for expr in match_expressions {
                        let value = pod_labels.get(&expr.key);
                        // invalid match, we don't want this pod; by the Kubernetes spec, we only
                        // want things that match BOTH all values AND all 
                        if !expr.match_value(value) {
                            continue 'outer;
                        }
                }
            }
            let node_name = pod
                .spec
                .and_then(|spec| spec.node_name)
                .ok_or(anyhow!("Unable to get pod.spec.node_name"))?;
            if node_names.contains(&node_name) { // do not re-query a node already seen
                continue;
            }
            let node = nodes.get(&node_name).await?;
            node_names.push(node_name);
            let node_addresses = node
                .status
                .and_then(|status| status.addresses)
                .ok_or(anyhow!("Unable to get node.status.addresses"))?;
            for node_ip in node_addresses.iter().filter(|addr| addr.type_ == "ExternalIP") {
                if !ips.contains(&node_ip.address) {
                    // do not add the same IP if it has been seen before; this is not likely given
                    // the node_names de-duplication above, but it may be possible that multiple
                    // nodes share a floating IP for some reason. this is for the most part a
                    // sanity check, and will not be practical for most instances.
                    ips.push(node_ip.address.clone());
                }
            }
        }

        Ok(ips)
    }

    /// Watch over changes to all Pods to determine whether or not a new IP address has been
    /// added or whether an old IP address no longer hosts an instance of the pod.
    async fn on_value_change(&self, opts: &Opts, provider_config: &mut ProviderConfig,
                            record_builder: &mut RecordBuilder) -> Result<()> {
        // TODO: async watcher over PodSelector, call f() every time a new Node is added or an
        // old Node is removed.
        let list_params = self.get_list_parameters();
        let mut current_values = self.get_values(opts).await?;
        current_values.sort();
        let pods: Api<Pod> = Api::all(Client::try_default().await?);
        let mut stream = pods.watch(&list_params, "0").await?.boxed();
        while let Some(pod_status) = stream.try_next().await? {
            match pod_status {
                | WatchEvent::Added(_)
                | WatchEvent::Deleted(_) => {
                    // Regardless of the event, we need to re-sync the list of Pods and call
                    // RecordChange on any added/removed values. We do this generically rather
                    // than determining the IP that a Pod exists on, because multiple Pods can
                    // exist on the same machine. If we were to indiscriminantly remove the IP
                    // address, this could lead to moving from two Pods to one, but the IP still
                    // being removed.
                    let mut new_values = self.get_values(&opts).await?;
                    new_values.sort();
                    let max = std::cmp::max(current_values.len(), new_values.len());
                    let current = 0;
                    let (mut left_index, mut right_index) = (0, 0);
                    while current < max {
                        // Check if old_values differs from new_values. If new_values does not
                        // contain the value at the current index, it was removed. If old_values
                        // does not contain the value at the current index, it was added.
                        // We do not have a guarantee that multiple addresses were not added at
                        // once, and while I don't think it's possible, better safe than sorry.
                        let ip_left = current_values.get(left_index);
                        let ip_right = new_values.get(right_index);
                        let ev = match (ip_left, ip_right) {
                            (None, None) => {
                                break
                            },
                            (Some(left), None) => {
                                // Old value exists, new value does not. Increment left
                                // index and delete record.
                                left_index += 1;
                                Some(RecordChange::Remove(left))
                            },
                            (None, Some(right)) => {
                                // New value exists, old value does not. Increment right
                                // index and add record.
                                Some(RecordChange::Add(right))
                            },
                            (Some(left), Some(right)) => {
                                // If the value at the left is less than the value at the right,
                                // that means that when sorted, a similar value on the right was
                                // not found. Similarly, if a value at the left is greater than the
                                // value at the right, a similar value on the left was not found.
                                // Because the values on the left are "old" records, matching
                                // values on the right not being found means that those records
                                // should be removed. Because the values on the right are "new"
                                // records, matching values on the left not being found means that
                                // those records should be created.
                                if left < right {
                                    // See above; old exists, new doesn't
                                    left_index += 1;
                                    Some(RecordChange::Remove(left))
                                } else if left > right {
                                    // See above; new exists, old doesn't
                                    right_index += 1;
                                    Some(RecordChange::Add(right))
                                } else {
                                    // Both indexes are the same. Increment each index by one, and
                                    // do not produce an event.
                                    left_index += 1;
                                    right_index += 1;
                                    None
                                }
                            }
                        }; // let ev
                        if let Some(event) = ev {
                            // pass
                            let provider: &mut dyn ProviderBackend = provider_config.deref_mut();
                            match event {
                                RecordChange::Add(value) => {
                                    let new_value = value.clone();
                                    let record = record_builder
                                        .value(new_value)
                                        .ttl(5) // ::TODO:: custom TTL
                                        .build()?;
                                    provider.add_record(&record.zone, &record).await?;
                                },
                                RecordChange::Remove(value) => {
                                    let new_value = value.clone();
                                    let record = record_builder
                                        .value(new_value)
                                        .ttl(5) // ::TODO:: custom TTL
                                        .build()?;
                                    provider.delete_record(&record.zone, &record).await?;
                                }
                            }
                        }
                    }
                },
                | WatchEvent::Modified(_)
                | WatchEvent::Bookmark(_) => {
                    // Do nothing. Pods being Modified can't change Nodes. I  don't even think
                    // Pods /can/ be Modified.
                },
                WatchEvent::Error(e) => {
                    // We got an error when watching. While this shouldn't happen often, it should
                    // be bubbled up and handled by the controller, which will then restart the
                    // watcher.
                    return Err(e.into())
                }
            }
            dbg!(pod_status);
        }
        Err(anyhow!("test"))
    }
}

trait_enum::trait_enum! {
    #[derive(Clone, Serialize, Deserialize, Debug)]
    pub enum RecordValueFrom: RecordValueCollector {
        #[serde(rename = "podSelector")]
        PodSelector,
    }
}

#[derive(CustomResource, Clone, Deserialize, Serialize, Debug)]
#[kube(group="syntixi.io", version="v1alpha1", namespaced)]
pub struct RecordSpec {
    fqdn: String,
    ttl: u32,
    #[serde(rename = "type")]
    type_: String,
    value: Option<Vec<String>>,
    #[serde(rename = "valueFrom")]
    pub value_from: Option<RecordValueFrom>,
}
