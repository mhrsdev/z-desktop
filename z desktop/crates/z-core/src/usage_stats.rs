//! Minimal token-per-task ledger (tok-019) — pure accounting, no I/O.
//!
//! The orchestrator charges tokens to task ids as provider responses report
//! usage; this ledger just accumulates and answers totals. Durable
//! persistence (journal records, replay) is deliberately out of scope here.

/// Per-task budget ceiling used by [`over_budget`] (orch-019).
pub const ORCH_TOKEN_BUDGET_PER_TASK: u64 = 200_000;

/// Tokens charged to one task id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskTokenUsage {
    pub task_id: String,
    pub tokens_used: u64,
}

impl TaskTokenUsage {
    /// True when the task has consumed more than the per-task budget.
    pub fn over_budget(&self) -> bool {
        over_budget(self)
    }
}

/// Accumulating token ledger keyed by task id. Insertion order is kept so
/// `top_n` ties break stably by first charge.
#[derive(Debug, Default, Clone)]
pub struct TokenLedger {
    entries: Vec<TaskTokenUsage>,
}

impl TokenLedger {
    /// Accumulate `tokens` onto `task_id`, creating the entry on first charge.
    pub fn charge(&mut self, task_id: &str, tokens: u64) {
        if let Some(e) = self.entries.iter_mut().find(|e| e.task_id == task_id) {
            e.tokens_used = e.tokens_used.saturating_add(tokens);
        } else {
            self.entries.push(TaskTokenUsage {
                task_id: task_id.to_string(),
                tokens_used: tokens,
            });
        }
    }

    /// Sum across all tasks.
    pub fn total(&self) -> u64 {
        self.entries.iter().map(|e| e.tokens_used).sum()
    }

    /// Total charged to one task (0 if unknown).
    pub fn for_task(&self, id: &str) -> u64 {
        self.entries
            .iter()
            .find(|e| e.task_id == id)
            .map(|e| e.tokens_used)
            .unwrap_or(0)
    }

    /// Top `n` tasks by usage, descending; ties stable by insertion order.
    pub fn top_n(&self, n: usize) -> Vec<(String, u64)> {
        let mut idx: Vec<usize> = (0..self.entries.len()).collect();
        // Stable sort by key keeps insertion order for equal usage.
        idx.sort_by(|&a, &b| self.entries[b].tokens_used.cmp(&self.entries[a].tokens_used));
        idx.into_iter()
            .take(n)
            .map(|i| (self.entries[i].task_id.clone(), self.entries[i].tokens_used))
            .collect()
    }
}

/// Free-function form of [`TaskTokenUsage::over_budget`]: strictly greater
/// than the budget counts as over (exactly at budget is still within).
pub fn over_budget(usage: &TaskTokenUsage) -> bool {
    usage.tokens_used > ORCH_TOKEN_BUDGET_PER_TASK
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn charge_accumulates_across_calls() {
        let mut l = TokenLedger::default();
        l.charge("t1", 100);
        l.charge("t1", 250);
        assert_eq!(l.for_task("t1"), 350);
        assert_eq!(l.total(), 350);
    }

    #[test]
    fn top_n_orders_desc_and_caps() {
        let mut l = TokenLedger::default();
        l.charge("a", 10);
        l.charge("b", 30);
        l.charge("c", 20);
        l.charge("d", 30); // tie with b -> b first (insertion order)
        assert_eq!(
            l.top_n(2),
            vec![("b".to_string(), 30), ("d".to_string(), 30)]
        );
        assert_eq!(l.top_n(0), Vec::new());
        assert_eq!(l.top_n(99).len(), 4);
    }

    #[test]
    fn over_budget_boundary_is_strict() {
        let at = TaskTokenUsage { task_id: "t".into(), tokens_used: ORCH_TOKEN_BUDGET_PER_TASK };
        let over = TaskTokenUsage {
            task_id: "t".into(),
            tokens_used: ORCH_TOKEN_BUDGET_PER_TASK + 1,
        };
        let under = TaskTokenUsage { task_id: "t".into(), tokens_used: ORCH_TOKEN_BUDGET_PER_TASK - 1 };
        assert!(!over_budget(&at));
        assert!(over_budget(&over));
        assert!(!over_budget(&under));
    }

    #[test]
    fn distinct_tasks_tracked_independently() {
        let mut l = TokenLedger::default();
        l.charge("x", 5);
        l.charge("y", 7);
        l.charge("x", 1);
        assert_eq!(l.for_task("x"), 6);
        assert_eq!(l.for_task("y"), 7);
        assert_eq!(l.for_task("missing"), 0);
        assert_eq!(l.total(), 13);
    }
}
