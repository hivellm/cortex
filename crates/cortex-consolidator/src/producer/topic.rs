//! Phase11j §2.5 — Topic producer.
//!
//! Input: turn-vector embeddings clustered via HDBSCAN
//! (`min_cluster_size = 3`) per repo. Output: one consolidation per
//! cluster with `grain = Topic`. The §2.1 skeleton fixes the input
//! shape + cluster-id contract so §2.5 can drop in the HDBSCAN
//! integration alongside the live producer body.

/// One cluster the orchestrator hands the topic producer. The
/// orchestrator runs HDBSCAN once per repo and emits one of these
/// per cluster whose size ≥ `MIN_CLUSTER_SIZE`.
#[derive(Debug, Clone)]
pub struct TopicCluster {
    /// Stable label the producer derives from the cluster centroid
    /// (typically a noun phrase). Drives `scope = Topic(_)`.
    pub label: String,
    /// Repo the cluster lives in. Topic clusters never cross repos.
    pub repo: String,
    /// Session ids inside the cluster, ordered by `occurred_at` of
    /// the centroid turn.
    pub session_ids: Vec<String>,
}

/// Phase11j §2.5 — minimum cluster size HDBSCAN runs with.
pub const MIN_CLUSTER_SIZE: usize = 3;

impl TopicCluster {
    /// Sanity check before invoking the summariser.
    pub fn ensure_min_size(&self) -> Result<(), super::ProducerError> {
        if self.session_ids.len() < MIN_CLUSTER_SIZE {
            return Err(super::ProducerError::EmptyInput(format!(
                "topic cluster {:?} has {} sessions, below MIN_CLUSTER_SIZE = {}",
                self.label,
                self.session_ids.len(),
                MIN_CLUSTER_SIZE
            )));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ensure_min_size_rejects_undersized_cluster() {
        let c = TopicCluster {
            label: "hnsw".into(),
            repo: "cortex".into(),
            session_ids: vec!["a".into(), "b".into()],
        };
        c.ensure_min_size().expect_err("2 < 3");
    }

    #[test]
    fn ensure_min_size_accepts_cluster_at_threshold() {
        let c = TopicCluster {
            label: "hnsw".into(),
            repo: "cortex".into(),
            session_ids: vec!["a".into(), "b".into(), "c".into()],
        };
        c.ensure_min_size().expect("3 >= 3");
    }
}
