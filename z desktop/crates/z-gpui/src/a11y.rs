//! Accessibility: the semantic tree that sits beside the visual scene.
//!
//! A renderer that draws rectangles and glyphs has no idea what any of them
//! *mean*. A screen reader needs to know that a particular rounded rectangle is
//! a button called "Send", that a list has nine items, that a checklist row is
//! in progress. None of that can be inferred from a quad.
//!
//! So the view declares semantics explicitly, in parallel with the visuals.
//! Declaring is more work than inferring, and it is the only approach that can
//! actually be correct — an inferred label is a guess, and a wrong label is
//! worse than none.
//!
//! This module also owns **focus order**, which is a semantic question rather
//! than a geometric one: the order a keyboard should walk the interface is the
//! order the interface is meant to be read, not the order rectangles happen to
//! sit in memory.

use crate::geometry::{Point, Rect};

/// Stable identity for one accessible element.
///
/// Assigned by the view from a stable source — a panel id, a nav item, an index
/// — never from iteration order, so focus survives a rebuild.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NodeId(pub u64);

impl NodeId {
    /// Derive an id from a namespace and an index. Keeps ids stable across
    /// frames without a global counter.
    pub const fn new(namespace: u32, index: u32) -> Self {
        NodeId(((namespace as u64) << 32) | index as u64)
    }

    pub const ROOT: NodeId = NodeId(0);
}

/// What an element is. Deliberately a small set: every role here maps onto
/// something every platform's accessibility API understands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// The application window itself.
    Window,
    /// A titled region — a panel, a toolbar.
    Group,
    /// Static text with no interaction.
    Label,
    Button,
    /// A control that toggles between two states.
    Toggle,
    /// One entry in a tab strip.
    Tab,
    TabList,
    List,
    ListItem,
    /// A multi-line text field.
    TextInput,
    /// Determinate progress, with a value from 0.0 to 1.0.
    ProgressIndicator,
    /// A scrollable region.
    ScrollArea,
    /// One message in a conversation.
    Article,
}

impl Role {
    /// Whether an element of this role is expected to carry a name.
    ///
    /// An unnamed button is unusable with a screen reader; an unnamed group is
    /// merely unhelpful.
    pub fn requires_label(self) -> bool {
        matches!(self, Role::Button | Role::Toggle | Role::Tab | Role::TextInput | Role::Label)
    }

    /// Whether keyboard focus can land here by default.
    pub fn is_focusable_by_default(self) -> bool {
        matches!(self, Role::Button | Role::Toggle | Role::Tab | Role::TextInput)
    }
}

/// Runtime state of an element, beyond its role.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct NodeState {
    pub selected: bool,
    pub disabled: bool,
    /// `None` for elements that do not expand.
    pub expanded: Option<bool>,
    /// Work is in progress here. Announced so a user is not left waiting in
    /// silence.
    pub busy: bool,
    /// Value from 0.0 to 1.0 for [`Role::ProgressIndicator`].
    pub value: Option<f32>,
}

/// One element of the semantic tree.
#[derive(Debug, Clone, PartialEq)]
pub struct AccessNode {
    pub id: NodeId,
    pub role: Role,
    /// What a screen reader announces. Must be meaningful on its own: "Send",
    /// not "button"; "Improve token validation, in progress", not "row 3".
    pub label: String,
    /// Extra detail, announced after the label.
    pub description: Option<String>,
    pub bounds: Rect,
    pub focusable: bool,
    pub state: NodeState,
    pub children: Vec<NodeId>,
}

impl AccessNode {
    pub fn new(id: NodeId, role: Role, label: impl Into<String>, bounds: Rect) -> Self {
        Self {
            id,
            role,
            label: label.into(),
            description: None,
            bounds,
            focusable: role.is_focusable_by_default(),
            state: NodeState::default(),
            children: Vec::new(),
        }
    }

    pub fn described(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    pub fn focusable(mut self, focusable: bool) -> Self {
        self.focusable = focusable;
        self
    }

    pub fn selected(mut self, selected: bool) -> Self {
        self.state.selected = selected;
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.state.disabled = disabled;
        self
    }

    pub fn expanded(mut self, expanded: bool) -> Self {
        self.state.expanded = Some(expanded);
        self
    }

    pub fn busy(mut self, busy: bool) -> Self {
        self.state.busy = busy;
        self
    }

    pub fn valued(mut self, value: f32) -> Self {
        self.state.value = Some(value.clamp(0.0, 1.0));
        self
    }

    pub fn with_children(mut self, children: impl IntoIterator<Item = NodeId>) -> Self {
        self.children = children.into_iter().collect();
        self
    }

    /// Whether keyboard focus can actually land here right now.
    pub fn can_take_focus(&self) -> bool {
        self.focusable && !self.state.disabled && !self.bounds.is_empty()
    }

    /// The full announcement: label, then state, then description.
    pub fn announcement(&self) -> String {
        let mut parts = vec![self.label.clone()];

        if self.state.disabled {
            parts.push("disabled".into());
        }
        if self.state.selected {
            parts.push("selected".into());
        }
        if let Some(expanded) = self.state.expanded {
            parts.push(if expanded { "expanded".into() } else { "collapsed".into() });
        }
        if self.state.busy {
            parts.push("busy".into());
        }
        if let Some(value) = self.state.value {
            parts.push(format!("{}%", (value * 100.0).round() as u32));
        }
        if let Some(description) = &self.description {
            parts.push(description.clone());
        }

        parts.join(", ")
    }
}

/// The semantic tree for one frame, plus where focus currently sits.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AccessTree {
    nodes: Vec<AccessNode>,
    focused: Option<NodeId>,
}

impl AccessTree {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, node: AccessNode) {
        self.nodes.push(node);
    }

    pub fn nodes(&self) -> &[AccessNode] {
        &self.nodes
    }

    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    pub fn get(&self, id: NodeId) -> Option<&AccessNode> {
        self.nodes.iter().find(|node| node.id == id)
    }

    pub fn focused(&self) -> Option<NodeId> {
        self.focused
    }

    /// Move focus to `id` if it can take it. Returns whether focus moved.
    pub fn focus(&mut self, id: NodeId) -> bool {
        if self.get(id).is_some_and(|node| node.can_take_focus()) {
            self.focused = Some(id);
            true
        } else {
            false
        }
    }

    pub fn clear_focus(&mut self) {
        self.focused = None;
    }

    /// Elements a keyboard can reach, in declaration order.
    ///
    /// Declaration order is deliberate: the view declares nodes in reading
    /// order, so Tab follows the order the interface is meant to be read rather
    /// than a geometric sort, which gets multi-column layouts wrong.
    pub fn focus_order(&self) -> Vec<NodeId> {
        self.nodes.iter().filter(|n| n.can_take_focus()).map(|n| n.id).collect()
    }

    /// The topmost focusable control under a logical-pixel pointer position.
    ///
    /// Nodes are declared in paint order. Walking backwards therefore gives an
    /// overlay such as the floating Tools bubble precedence over the surface
    /// beneath it, while decorative or disabled nodes never consume a click.
    pub fn focusable_at(&self, point: Point) -> Option<NodeId> {
        self.nodes
            .iter()
            .rev()
            .find(|node| node.can_take_focus() && node.bounds.contains(point))
            .map(|node| node.id)
    }

    /// Move focus forward, wrapping. Returns the new focus.
    pub fn focus_next(&mut self) -> Option<NodeId> {
        self.step_focus(true)
    }

    /// Move focus backward, wrapping.
    pub fn focus_previous(&mut self) -> Option<NodeId> {
        self.step_focus(false)
    }

    fn step_focus(&mut self, forward: bool) -> Option<NodeId> {
        let order = self.focus_order();
        if order.is_empty() {
            self.focused = None;
            return None;
        }

        let next = match self.focused.and_then(|id| order.iter().position(|o| *o == id)) {
            Some(index) => {
                let count = order.len();
                // Wrapping, never trapping: Tab from the last element returns to
                // the first rather than stopping.
                if forward {
                    (index + 1) % count
                } else {
                    (index + count - 1) % count
                }
            }
            // Focus was nowhere, or on something that has since gone away.
            None => {
                if forward {
                    0
                } else {
                    order.len() - 1
                }
            }
        };

        self.focused = Some(order[next]);
        self.focused
    }

    /// Bounds of the focused element, for drawing the focus ring.
    pub fn focus_bounds(&self) -> Option<Rect> {
        self.focused.and_then(|id| self.get(id)).map(|node| node.bounds)
    }

    /// Elements whose role demands a label but have none.
    ///
    /// An icon-only button with no name is invisible to a screen reader, and
    /// nothing about the rendered pixels reveals the omission — which is why
    /// this is a check rather than a code review item.
    pub fn unlabelled(&self) -> Vec<&AccessNode> {
        self.nodes
            .iter()
            .filter(|node| node.role.requires_label() && node.label.trim().is_empty())
            .collect()
    }

    /// Ids declared more than once. Duplicates make focus jump unpredictably.
    pub fn duplicate_ids(&self) -> Vec<NodeId> {
        let mut seen = std::collections::BTreeSet::new();
        let mut duplicates = std::collections::BTreeSet::new();
        for node in &self.nodes {
            if !seen.insert(node.id) {
                duplicates.insert(node.id);
            }
        }
        duplicates.into_iter().collect()
    }

    /// Children referenced by a node but never declared.
    pub fn dangling_children(&self) -> Vec<NodeId> {
        let declared: std::collections::BTreeSet<NodeId> =
            self.nodes.iter().map(|n| n.id).collect();
        let mut dangling = std::collections::BTreeSet::new();
        for node in &self.nodes {
            for child in &node.children {
                if !declared.contains(child) {
                    dangling.insert(*child);
                }
            }
        }
        dangling.into_iter().collect()
    }

    pub fn clear(&mut self) {
        self.nodes.clear();
        // Focus is kept: rebuilding the tree each frame must not drop the
        // user's place. It is validated on the next focus move instead.
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn button(index: u32, label: &str) -> AccessNode {
        AccessNode::new(
            NodeId::new(1, index),
            Role::Button,
            label,
            Rect::new(index as f32 * 40.0, 0.0, 32.0, 32.0),
        )
    }

    fn tree(labels: &[&str]) -> AccessTree {
        let mut tree = AccessTree::new();
        for (i, label) in labels.iter().enumerate() {
            tree.push(button(i as u32, label));
        }
        tree
    }

    #[test]
    fn ids_derived_from_a_namespace_and_index_are_distinct() {
        assert_ne!(NodeId::new(1, 0), NodeId::new(2, 0));
        assert_ne!(NodeId::new(1, 0), NodeId::new(1, 1));
        assert_eq!(NodeId::new(3, 7), NodeId::new(3, 7), "ids must be stable across frames");
    }

    #[test]
    fn interactive_roles_are_focusable_by_default() {
        for role in [Role::Button, Role::Toggle, Role::Tab, Role::TextInput] {
            assert!(role.is_focusable_by_default(), "{role:?} should take focus");
        }
        for role in [Role::Label, Role::Group, Role::Article] {
            assert!(!role.is_focusable_by_default(), "{role:?} should not take focus");
        }
    }

    #[test]
    fn tab_walks_declaration_order_and_wraps() {
        let mut tree = tree(&["one", "two", "three"]);

        assert_eq!(tree.focus_next(), Some(NodeId::new(1, 0)));
        assert_eq!(tree.focus_next(), Some(NodeId::new(1, 1)));
        assert_eq!(tree.focus_next(), Some(NodeId::new(1, 2)));
        assert_eq!(tree.focus_next(), Some(NodeId::new(1, 0)), "focus must wrap, never trap");
    }

    #[test]
    fn shift_tab_walks_backwards_and_wraps() {
        let mut tree = tree(&["one", "two", "three"]);

        assert_eq!(
            tree.focus_previous(),
            Some(NodeId::new(1, 2)),
            "from nowhere, start at the end"
        );
        assert_eq!(tree.focus_previous(), Some(NodeId::new(1, 1)));
        assert_eq!(tree.focus_previous(), Some(NodeId::new(1, 0)));
        assert_eq!(tree.focus_previous(), Some(NodeId::new(1, 2)), "and wrap");
    }

    #[test]
    fn disabled_elements_are_skipped_by_the_keyboard() {
        let mut tree = AccessTree::new();
        tree.push(button(0, "one"));
        tree.push(button(1, "two").disabled(true));
        tree.push(button(2, "three"));

        assert_eq!(tree.focus_order(), vec![NodeId::new(1, 0), NodeId::new(1, 2)]);
        tree.focus_next();
        assert_eq!(tree.focus_next(), Some(NodeId::new(1, 2)), "the disabled control was skipped");
    }

    #[test]
    fn a_zero_sized_element_cannot_take_focus() {
        let mut tree = AccessTree::new();
        tree.push(AccessNode::new(NodeId::new(1, 0), Role::Button, "hidden", Rect::ZERO));
        assert!(tree.focus_order().is_empty());
        assert!(!tree.focus(NodeId::new(1, 0)));
    }

    #[test]
    fn hit_testing_picks_the_topmost_focusable_control() {
        let mut tree = AccessTree::new();
        tree.push(AccessNode::new(
            NodeId::new(1, 0),
            Role::Button,
            "base",
            Rect::new(0.0, 0.0, 40.0, 40.0),
        ));
        tree.push(AccessNode::new(
            NodeId::new(1, 1),
            Role::Button,
            "overlay",
            Rect::new(10.0, 10.0, 40.0, 40.0),
        ));

        assert_eq!(tree.focusable_at(Point::new(20.0, 20.0)), Some(NodeId::new(1, 1)));
    }

    #[test]
    fn hit_testing_ignores_disabled_and_noninteractive_nodes() {
        let mut tree = AccessTree::new();
        tree.push(AccessNode::new(
            NodeId::new(1, 0),
            Role::Button,
            "enabled",
            Rect::new(0.0, 0.0, 40.0, 40.0),
        ));
        tree.push(
            AccessNode::new(
                NodeId::new(1, 1),
                Role::Button,
                "disabled",
                Rect::new(10.0, 10.0, 20.0, 20.0),
            )
            .disabled(true),
        );
        tree.push(AccessNode::new(
            NodeId::new(1, 2),
            Role::Label,
            "label",
            Rect::new(10.0, 10.0, 20.0, 20.0),
        ));

        assert_eq!(tree.focusable_at(Point::new(20.0, 20.0)), Some(NodeId::new(1, 0)));
        assert_eq!(tree.focusable_at(Point::new(80.0, 80.0)), None);
    }

    #[test]
    fn focusing_something_unfocusable_is_refused_rather_than_silently_accepted() {
        let mut tree = AccessTree::new();
        tree.push(AccessNode::new(
            NodeId::new(1, 0),
            Role::Label,
            "just text",
            Rect::new(0.0, 0.0, 100.0, 20.0),
        ));
        assert!(!tree.focus(NodeId::new(1, 0)));
        assert_eq!(tree.focused(), None);
    }

    #[test]
    fn a_tree_with_nothing_focusable_does_not_loop_forever() {
        let mut tree = AccessTree::new();
        tree.push(AccessNode::new(
            NodeId::new(1, 0),
            Role::Label,
            "text",
            Rect::new(0.0, 0.0, 10.0, 10.0),
        ));
        assert_eq!(tree.focus_next(), None);
        assert_eq!(tree.focus_previous(), None);
    }

    #[test]
    fn focus_survives_the_tree_being_rebuilt() {
        // The tree is rebuilt every frame; dropping focus each time would make
        // the keyboard unusable.
        let mut tree = tree(&["one", "two"]);
        tree.focus_next();
        let before = tree.focused();

        tree.clear();
        for (i, label) in ["one", "two"].iter().enumerate() {
            tree.push(button(i as u32, label));
        }

        assert_eq!(tree.focused(), before);
    }

    #[test]
    fn focus_on_a_vanished_element_recovers_rather_than_sticking() {
        let mut tree = tree(&["one", "two"]);
        tree.focus(NodeId::new(1, 1));

        // The element disappears — a panel closed, a list shortened.
        let mut rebuilt = AccessTree::new();
        rebuilt.push(button(0, "one"));
        rebuilt.focus_next();

        assert_eq!(rebuilt.focused(), Some(NodeId::new(1, 0)));
    }

    #[test]
    fn an_icon_only_control_without_a_name_is_reported() {
        let mut tree = AccessTree::new();
        tree.push(button(0, "Send"));
        tree.push(button(1, "   "));
        tree.push(AccessNode::new(
            NodeId::new(1, 2),
            Role::Group,
            "",
            Rect::new(0.0, 0.0, 10.0, 10.0),
        ));

        let missing = tree.unlabelled();
        assert_eq!(missing.len(), 1, "only roles that require a name should be flagged");
        assert_eq!(missing[0].id, NodeId::new(1, 1));
    }

    #[test]
    fn duplicate_ids_are_reported() {
        let mut tree = AccessTree::new();
        tree.push(button(0, "one"));
        tree.push(button(0, "clash"));
        assert_eq!(tree.duplicate_ids(), vec![NodeId::new(1, 0)]);
    }

    #[test]
    fn a_child_that_was_never_declared_is_reported() {
        let mut tree = AccessTree::new();
        tree.push(
            AccessNode::new(
                NodeId::new(1, 0),
                Role::Group,
                "panel",
                Rect::new(0.0, 0.0, 100.0, 100.0),
            )
            .with_children([NodeId::new(9, 9)]),
        );
        assert_eq!(tree.dangling_children(), vec![NodeId::new(9, 9)]);
    }

    #[test]
    fn the_announcement_carries_state_not_just_the_name() {
        let node = button(0, "Context").expanded(false).described("9 groups");
        let said = node.announcement();
        assert!(said.starts_with("Context"));
        assert!(said.contains("collapsed"));
        assert!(said.contains("9 groups"));
    }

    #[test]
    fn progress_is_announced_as_a_percentage() {
        let node = AccessNode::new(
            NodeId::new(2, 0),
            Role::ProgressIndicator,
            "Zero AI is making changes",
            Rect::new(0.0, 0.0, 100.0, 4.0),
        )
        .valued(0.6)
        .busy(true);

        let said = node.announcement();
        assert!(said.contains("busy"), "silence during work is the thing to avoid");
        assert!(said.contains("60%"));
    }

    #[test]
    fn a_progress_value_outside_the_range_is_clamped() {
        let node = AccessNode::new(
            NodeId::new(2, 0),
            Role::ProgressIndicator,
            "p",
            Rect::new(0.0, 0.0, 10.0, 4.0),
        )
        .valued(9.0);
        assert_eq!(node.state.value, Some(1.0));
    }

    #[test]
    fn focus_bounds_follow_the_focused_element() {
        let mut tree = tree(&["one", "two"]);
        tree.focus(NodeId::new(1, 1));
        assert_eq!(tree.focus_bounds(), Some(Rect::new(40.0, 0.0, 32.0, 32.0)));
    }
}
