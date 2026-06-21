//! Shell job table — tracks background and foreground jobs.

extern crate alloc;
use alloc::{string::String, vec::Vec};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobState { Running, Done }

/// One job entry.
struct Job {
    id:    usize,
    state: JobState,
    name:  String,
}

/// Active job registry.
#[derive(Default)]
pub struct Jobs {
    jobs:    Vec<Job>,
    next_id: usize,
}

impl Jobs {
    pub fn new() -> Self { Self { jobs: Vec::new(), next_id: 1 } }

    /// Add a new running job and return its job ID.
    pub fn add(&mut self, name: &str) -> usize {
        let id = self.next_id;
        self.next_id += 1;
        self.jobs.push(Job { id, state: JobState::Running, name: String::from(name) });
        id
    }

    /// Update the state of a job by ID.
    pub fn set_state(&mut self, id: usize, state: JobState) {
        if let Some(j) = self.jobs.iter_mut().find(|j| j.id == id) {
            j.state = state;
        }
    }

    /// Remove all completed jobs.
    pub fn reap_done(&mut self) {
        self.jobs.retain(|j| j.state != JobState::Done);
    }

    /// Iterate (id, state, name) for all jobs.
    pub fn list(&self) -> impl Iterator<Item = (usize, JobState, &str)> {
        self.jobs.iter().map(|j| (j.id, j.state, j.name.as_str()))
    }
}
