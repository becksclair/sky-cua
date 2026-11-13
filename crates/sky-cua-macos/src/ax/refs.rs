//! `backend_ref` strings and the bounded store of retained element handles
//! they address.
//!
//! A reference names one element of one tree read:
//! `ax:pid=<pid>;gen=<read generation>;idx=<element_index>`. Generations are
//! process-unique, so a reference never silently resolves to an element of a
//! different read.

use std::collections::VecDeque;

const PREFIX: &str = "ax:";

/// Tree reads retained per target process. More than one so an action can
/// still address an element of the snapshot taken just before a verification
/// snapshot.
pub(crate) const READS_PER_PROCESS: usize = 3;
/// Tree reads retained across all processes.
pub(crate) const READS_TOTAL: usize = 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct AxRef {
    pub pid: i32,
    pub generation: u64,
    pub index: usize,
}

impl AxRef {
    pub(crate) fn format(&self) -> String {
        format!(
            "{PREFIX}pid={};gen={};idx={}",
            self.pid, self.generation, self.index
        )
    }

    pub(crate) fn parse(value: &str) -> Option<Self> {
        let rest = value.strip_prefix(PREFIX)?;
        let (mut pid, mut generation, mut index) = (None, None, None);
        for part in rest.split(';') {
            let (key, value) = part.split_once('=')?;
            match key {
                "pid" => pid = value.parse().ok(),
                "gen" => generation = value.parse().ok(),
                "idx" => index = value.parse().ok(),
                _ => return None,
            }
        }
        Some(Self {
            pid: pid?,
            generation: generation?,
            index: index?,
        })
    }
}

pub(crate) fn is_ax_ref(value: &str) -> bool {
    AxRef::parse(value).is_some()
}

/// Retained handles of recent tree reads, oldest first.
#[derive(Debug)]
pub(crate) struct RefStore<H> {
    reads: VecDeque<RetainedRead<H>>,
}

#[derive(Debug)]
struct RetainedRead<H> {
    pid: i32,
    generation: u64,
    handles: Vec<H>,
}

impl<H> Default for RefStore<H> {
    fn default() -> Self {
        Self {
            reads: VecDeque::new(),
        }
    }
}

impl<H: Clone> RefStore<H> {
    pub(crate) fn insert(&mut self, pid: i32, generation: u64, handles: Vec<H>) {
        self.reads.push_back(RetainedRead {
            pid,
            generation,
            handles,
        });
        while self.reads.iter().filter(|read| read.pid == pid).count() > READS_PER_PROCESS {
            if let Some(oldest) = self.reads.iter().position(|read| read.pid == pid) {
                self.reads.remove(oldest);
            }
        }
        while self.reads.len() > READS_TOTAL {
            self.reads.pop_front();
        }
    }

    pub(crate) fn get(&self, reference: &AxRef) -> Option<H> {
        self.reads
            .iter()
            .rev()
            .find(|read| read.pid == reference.pid && read.generation == reference.generation)
            .and_then(|read| read.handles.get(reference.index))
            .cloned()
    }

    pub(crate) fn clear(&mut self) {
        self.reads.clear();
    }

    pub(crate) fn len(&self) -> usize {
        self.reads.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn references_round_trip_and_reject_foreign_strings() {
        let reference = AxRef {
            pid: 900,
            generation: 7,
            index: 12,
        };
        let text = reference.format();
        assert_eq!(text, "ax:pid=900;gen=7;idx=12");
        assert_eq!(AxRef::parse(&text), Some(reference));
        assert!(is_ax_ref(&text));
        assert!(!is_ax_ref("uia:hwnd=0x2a;path=0/1"));
        assert!(!is_ax_ref(":1.42:/org/a11y/atspi/accessible/12"));
        assert!(!is_ax_ref("ax:pid=1;gen=2"));
        assert!(!is_ax_ref("ax:pid=1;gen=2;idx=x"));
        assert!(!is_ax_ref("ax:pid=1;gen=2;idx=3;extra=4"));
    }

    #[test]
    fn store_keeps_recent_reads_per_process_and_overall() {
        let mut store = RefStore::default();
        for generation in 1..=5 {
            store.insert(1, generation, vec![generation * 10]);
        }
        assert_eq!(store.len(), READS_PER_PROCESS);
        let lookup = |store: &RefStore<u64>, pid, generation| {
            store.get(&AxRef {
                pid,
                generation,
                index: 0,
            })
        };
        assert_eq!(lookup(&store, 1, 2), None);
        assert_eq!(lookup(&store, 1, 3), Some(30));
        assert_eq!(lookup(&store, 1, 5), Some(50));

        for pid in 2..40 {
            store.insert(pid, 100 + pid as u64, vec![pid as u64]);
        }
        assert_eq!(store.len(), READS_TOTAL);
        assert_eq!(lookup(&store, 1, 5), None);
        assert_eq!(lookup(&store, 39, 139), Some(39));
        assert_eq!(
            store.get(&AxRef {
                pid: 39,
                generation: 139,
                index: 1
            }),
            None
        );
        store.clear();
        assert_eq!(store.len(), 0);
    }
}
