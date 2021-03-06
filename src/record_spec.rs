//! CRD and Code for records.syntixi.io/v1alpha1

// vim:set foldmethod=marker:

// {{{ imports
use std::ops::Deref;

use crate::cli::Opts;
use crate::providers::{
    util::{ProviderBackend, FullDomainName, ZoneDomainName, RecordBuilder, RecordType},
    ProviderConfig,
};

use futures::{
    future::FutureExt,
    pin_mut,
    select,
};

use anyhow::{anyhow, Result};
use k8s_openapi::api::core::v1::{Pod, Node};
use futures::{StreamExt, TryStreamExt};
use kube::{
    api::{Api, ListParams, WatchEvent, ObjectMeta},
    Client,
};
use kube_derive::CustomResource;
use serde::{Serialize, Deserialize};
// }}}

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
pub trait RecordValueCollector: Send + Sync {
    /// Return a default ListParams object. This should be overridden per-instance of
    /// RecordValueCollector if the matchLabels arguments can be passed through ListParams, as
    /// otherwise they will need to be parsed manually after acquiring all matching resources.
    fn get_list_parameters(&self) -> ListParams {
        ListParams::default()
    }

    /// Return the values that should be records for a RecordValueCollector. The ObjectMeta
    /// passed to the function should be the ObjectMeta of the Record. This is so namespaced
    /// attributes have an object with which to tie their reference.

    async fn get_values(&self, meta: &ObjectMeta) -> Result<Vec<String>>;

    /// Synchronize the remote Records with the correct Values. This should be run once, when
    /// initializing a RecordValueCollector, as further requests will introduce a large amount
    /// of traffic to the backend provider.
    ///
    /// This command can also be run in a timed loop during watch_values when a watcher over
    /// a resource is not available, but for the aforementioned reasons this is not recommended.
    async fn sync(&self, meta: &ObjectMeta, provider_config: &ProviderConfig,
                  record_builder: &mut RecordBuilder) -> Result<()>;

    /// Ensure by watching relevant objects (such as Pods) have a Record for every instance, and
    /// that if an object no longer has a connection to the relevant record (such as a Pod no
    /// longer existing on a Node) that the Record is removed. The ObjectMeta passed to the
    /// function should be the ObjectMeta of the Record. This is so namespaced attributes have an
    /// object with which to tie their reference.
    async fn watch_values(&self, meta: &ObjectMeta, provider_config: &ProviderConfig,
                          record_builder: &mut RecordBuilder) -> Result<Record>;
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
    /// Create a set of ListParams based on the match_labels values passed to the Record
    /// resource. List parameters are used to slim down the amount of values returned by
    /// the Kubernetes API, but come with the potential downside of relying on the Kubernetes
    /// API to filter by label.
    fn get_list_parameters(&self) -> ListParams {
        let mut list_params = ListParams::default();
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
    async fn get_values(&self, meta: &ObjectMeta) -> Result<Vec<String>> {
        let list_params = self.get_list_parameters();

        let pods: Api<Pod> = Api::namespaced(Client::try_default().await?,
                                             meta
                                                .namespace
                                                .as_ref()
                                                .ok_or(anyhow!("Missing meta.namespace"))?
                                                .as_str());
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
                        // want things that match BOTH all values AND all expressions.
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

    async fn sync(&self, meta: &ObjectMeta, provider_config: &ProviderConfig,
                  record_builder: &mut RecordBuilder) -> Result<()> {
        let values = self.get_values(meta).await?;
        let provider: &dyn ProviderBackend = provider_config.deref();
        provider.sync_records(record_builder, &values).await?;
        Ok(())
    }

    /// Watch over changes to all Pods to determine whether or not a new IP address has been
    /// added or whether an old IP address no longer hosts an instance of the pod.
    async fn watch_values(&self, meta: &ObjectMeta, provider_config: &ProviderConfig,
                          record_builder: &mut RecordBuilder) -> Result<Record> {
        let mut current_values = self.get_values(meta).await?;
        current_values.sort();

        let record_name: &str = meta.name.as_ref().ok_or(anyhow!("Missing record.meta.name"))?;
        let record_namespace: &str = meta
            .namespace
            .as_ref()
            .ok_or(anyhow!("Missing record.meta.namespace"))?;
        let record_list_params = ListParams::default();
        let records: Api<Record> = Api::namespaced(Client::try_default().await?,
                                                   record_namespace);
        let mut record_watcher = records.watch(&record_list_params, "0").await?.boxed().fuse();

        let list_params = self.get_list_parameters();
        let pods: Api<Pod> = Api::all(Client::try_default().await?);
        let mut pod_watcher = pods.watch(&list_params, "0").await?.boxed().fuse();

        loop {
            #[derive(Debug)]
            enum Event {
                Pod(WatchEvent<Pod>),
                Record(WatchEvent<Record>),
            }

            let event: Event = select! {
                pod_status_result = pod_watcher.try_next() => {
                    Event::Pod(match pod_status_result {
                        Ok(v) => match v {
                            Some(v) => v,
                            None => return Err(anyhow!("Found None")),
                        },
                        Err(e) => return Err(e.into()),
                    })
                },
                record_status_result = record_watcher.try_next() => {
                    Event::Record(match record_status_result {
                        Ok(v) => match v {
                            Some(v) => v,
                            None => return Err(anyhow!("Found None")),
                        },
                        Err(e) => return Err(e.into()),
                    })
                },
            };

            match event {
                Event::Pod(pod_status) => {
                    match pod_status {
                        | WatchEvent::Added(_)
                        | WatchEvent::Deleted(_) => {
                            // Regardless of the event, we need to re-sync the list of Pods and
                            // call RecordChange on any added/removed values. We do this
                            // generically rather than determining the IP that a Pod exists on,
                            // because multiple Pods can exist on the same machine. If we were to
                            // indiscriminantly remove the IP address, this could lead to moving
                            // from two Pods to one, but the IP still being removed.
                            let mut new_values = self.get_values(&meta).await?;
                            new_values.sort();
                            let (mut left_index, mut right_index) = (0, 0);
                            loop {
                                // Check if old_values differs from new_values. If new_values
                                // does not contain the value at the current index, it was removed.
                                // If old_values does not contain the value at the current index,
                                // it was added.  We do not have a guarantee that multiple
                                // addresses were not added at once, and while I don't think it's
                                // possible, better safe than sorry.
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
                                        // If the value at the left is less than the value at the
                                        // right, that means that when sorted, a similar value on
                                        // the right was not found. Similarly, if a value at the
                                        // left is greater than the value at the right, a similar
                                        // value on the left was not found.  Because the values
                                        // on the left are "old" records, matching values on the
                                        // right not being found means that those records should
                                        // be removed. Because the values on the right are "new"
                                        // records, matching values on the left not being found
                                        // means that those records should be created.
                                        if left < right {
                                            // See above; old exists, new doesn't
                                            left_index += 1;
                                            Some(RecordChange::Remove(left))
                                        } else if left > right {
                                            // See above; new exists, old doesn't
                                            right_index += 1;
                                            Some(RecordChange::Add(right))
                                        } else {
                                            // Both indexes are the same. Increment each index by
                                            // one, and do not produce an event.
                                            left_index += 1;
                                            right_index += 1;
                                            None
                                        }
                                    }
                                }; // let ev
                                if let Some(event) = ev {
                                    // pass
                                    let provider: &dyn ProviderBackend = provider_config.deref();
                                    match event {
                                        RecordChange::Add(value) => {
                                            let new_value = value.clone();
                                            let record = record_builder
                                                .clone()
                                                .value(new_value)
                                                .ttl(1) // ::TODO:: custom TTL
                                                .try_build()?;
                                            provider.add_record(&record.zone, &record).await?;
                                        },
                                        RecordChange::Remove(value) => {
                                            let new_value = value.clone();
                                            let record = record_builder
                                                .clone()
                                                .value(new_value)
                                                .ttl(1) // ::TODO:: custom TTL
                                                .try_build()?;
                                            provider.delete_record(&record.zone, &record).await?;
                                        }
                                    }
                                }
                            }
                            current_values = new_values;
                        },
                        | WatchEvent::Modified(_)
                        | WatchEvent::Bookmark(_) => {
                            // Do nothing. Pods being Modified can't change Nodes.
                        },
                        WatchEvent::Error(e) => {
                            // We got an error when watching. While this shouldn't happen often,
                            // it should be bubbled up and handled by the controller, which will
                            // then restart the watcher.
                            return Err(e.into())
                        },
                    }
                },
                Event::Record(record_status) => {
                    match record_status {
                        WatchEvent::Added(new) => {
                            // verify that live record matches the current record
                            if new.metadata.uid == meta.uid {
                                if (new.metadata.resource_version != meta.resource_version) {
                                    // The record was deleted in-between starting watch_values
                                    // and starting the actual watcher.
                                    return Ok(new)
                                }
                            }
                        },
                        | WatchEvent::Bookmark(_) => {
                            // do nothing
                        },
                        WatchEvent::Modified(modified) => {
                            if modified.metadata.uid == meta.uid {
                                return Ok(modified)
                            }
                        },
                        WatchEvent::Deleted(deleted) => {
                            if deleted.metadata.uid == meta.uid {
                                return Err(anyhow!("Record deleted"));
                            }
                        },
                        WatchEvent::Error(e) => {
                            return Err(e.into())
                        },
                    }
                },
            }
        }

        records.get(record_name.as_ref()).await.map_err(|x| x.into()) // cycle refresh
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
    pub fqdn: FullDomainName,
    pub ttl: u32,
    #[serde(rename = "type")]
    pub type_: RecordType,
    pub value: Option<Vec<String>>,
    #[serde(rename = "valueFrom")]
    pub value_from: Option<RecordValueFrom>,
}
