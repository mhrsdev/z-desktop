//! Handing the semantic tree to the platform.
//!
//! [`crate::a11y`] owns the model; this module is the adapter that turns it
//! into something UI Automation (Windows), AT-SPI (Linux) and NSAccessibility
//! (macOS) understand, via AccessKit.
//!
//! The split is the same one ADR-0012 makes for graphics: our model is ours,
//! and the external crate is an implementation detail confined to one file. No
//! `accesskit` type appears in any public signature outside this module, so
//! swapping the platform layer would not touch a single view.
//!
//! **Nothing here invents semantics.** Every label, role and state was declared
//! by the view. This code only translates.

use crate::a11y::{AccessNode, AccessTree, NodeId, Role};
use crate::geometry::Rect;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex, MutexGuard};
use winit::event::WindowEvent;
use winit::event_loop::ActiveEventLoop;
use winit::window::Window;

/// The root the platform tree hangs from.
///
/// AccessKit needs a single root; our tree is a flat list in reading order, so
/// the first `Window` node becomes the root and everything else becomes its
/// child. Flat is deliberate: nesting would have to be inferred from geometry,
/// and inference is what this design avoids.
const PLATFORM_ROOT: accesskit::NodeId = accesskit::NodeId(u64::MAX);

fn to_accesskit_id(id: NodeId) -> accesskit::NodeId {
    accesskit::NodeId(id.0)
}

fn to_accesskit_role(role: Role) -> accesskit::Role {
    match role {
        Role::Window => accesskit::Role::Window,
        Role::Group => accesskit::Role::Group,
        Role::Label => accesskit::Role::Label,
        Role::Button => accesskit::Role::Button,
        // `Switch` rather than `CheckBox`: the Performance toggle turns a thing
        // on and off, it does not mark a choice in a set.
        Role::Toggle => accesskit::Role::Switch,
        Role::Tab => accesskit::Role::Tab,
        Role::TabList => accesskit::Role::TabList,
        Role::List => accesskit::Role::List,
        Role::ListItem => accesskit::Role::ListItem,
        Role::TextInput => accesskit::Role::TextInput,
        Role::ProgressIndicator => accesskit::Role::ProgressIndicator,
        Role::ScrollArea => accesskit::Role::ScrollView,
        Role::Article => accesskit::Role::Article,
    }
}

/// AccessKit rectangles are corner-to-corner in physical pixels; ours are
/// origin-plus-size in logical pixels.
fn to_accesskit_rect(rect: Rect, scale: f32) -> accesskit::Rect {
    let scale = scale.max(0.01) as f64;
    accesskit::Rect {
        x0: rect.x as f64 * scale,
        y0: rect.y as f64 * scale,
        x1: rect.right() as f64 * scale,
        y1: rect.bottom() as f64 * scale,
    }
}

fn to_accesskit_node(node: &AccessNode, scale: f32) -> accesskit::Node {
    let mut out = accesskit::Node::new(to_accesskit_role(node.role));

    out.set_label(node.label.clone());
    out.set_bounds(to_accesskit_rect(node.bounds, scale));

    if let Some(description) = &node.description {
        out.set_description(description.clone());
    }

    if node.state.disabled {
        out.set_disabled();
    }
    if node.state.selected {
        out.set_selected(true);
    }
    if let Some(expanded) = node.state.expanded {
        out.set_expanded(expanded);
    }
    if node.state.busy {
        out.set_busy();
    }
    if let Some(value) = node.state.value {
        // A screen reader reads progress as a number in a range, so the range
        // has to be there too — a bare value is meaningless on its own.
        //
        // Rounded to a tenth of a percent: widening f32 to f64 leaves noise in
        // the low digits, and a screen reader announcing "60.00000238 percent"
        // is worse than useless.
        let percent = (value as f64 * 1000.0).round() / 10.0;
        out.set_numeric_value(percent);
        out.set_min_numeric_value(0.0);
        out.set_max_numeric_value(100.0);
    }

    // Only advertise focus where focus can actually land, or a screen reader
    // will offer the user somewhere it cannot go.
    if node.can_take_focus() {
        out.add_action(accesskit::Action::Focus);
        out.add_action(accesskit::Action::Click);
    }

    if !node.children.is_empty() {
        out.set_children(node.children.iter().copied().map(to_accesskit_id).collect::<Vec<_>>());
    }

    out
}

/// Translate the whole tree into one platform update.
///
/// `scale` is the window's scale factor: the platform works in physical pixels,
/// everything above the renderer works in logical ones.
pub fn to_tree_update(tree: &AccessTree, scale: f32) -> accesskit::TreeUpdate {
    let mut nodes: Vec<(accesskit::NodeId, accesskit::Node)> = Vec::with_capacity(tree.len() + 1);

    // Root: a container holding everything, in the order the view declared it.
    let mut root = accesskit::Node::new(accesskit::Role::Window);
    root.set_label("Zero");
    root.set_children(tree.nodes().iter().map(|n| to_accesskit_id(n.id)).collect::<Vec<_>>());
    nodes.push((PLATFORM_ROOT, root));

    for node in tree.nodes() {
        nodes.push((to_accesskit_id(node.id), to_accesskit_node(node, scale)));
    }

    accesskit::TreeUpdate {
        nodes,
        tree: Some(accesskit::Tree::new(PLATFORM_ROOT)),
        tree_id: accesskit::TreeId::ROOT,
        // Focus must name a node that exists in this update, or the platform
        // is left pointing at nothing.
        focus: tree.focused().map(to_accesskit_id).unwrap_or(PLATFORM_ROOT),
    }
}

/// An action the platform asked us to perform.
///
/// Translated out of AccessKit's vocabulary so the app never sees the external
/// type — the same boundary rule the renderer follows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccessRequest {
    /// Move keyboard focus to this element.
    Focus(NodeId),
    /// Activate it — the equivalent of a click or Enter.
    Activate(NodeId),
    /// Bring it into view.
    ScrollIntoView(NodeId),
}

impl AccessRequest {
    /// Translate a platform action request, or `None` for ones we do not handle.
    pub fn from_action(request: &accesskit::ActionRequest) -> Option<Self> {
        let id = NodeId(request.target_node.0);
        match request.action {
            accesskit::Action::Focus => Some(AccessRequest::Focus(id)),
            accesskit::Action::Click => Some(AccessRequest::Activate(id)),
            accesskit::Action::ScrollIntoView => Some(AccessRequest::ScrollIntoView(id)),
            _ => None,
        }
    }

    pub fn target(self) -> NodeId {
        match self {
            AccessRequest::Focus(id)
            | AccessRequest::Activate(id)
            | AccessRequest::ScrollIntoView(id) => id,
        }
    }
}

type LatestTree = Arc<Mutex<Option<(AccessTree, f32)>>>;
type PendingRequests = Arc<Mutex<VecDeque<AccessRequest>>>;
type WakeWindowLoop = Arc<dyn Fn() + Send + Sync>;

fn lock_unpoisoned<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

struct PlatformActivationHandler {
    latest: LatestTree,
}

impl accesskit::ActivationHandler for PlatformActivationHandler {
    fn request_initial_tree(&mut self) -> Option<accesskit::TreeUpdate> {
        let latest = lock_unpoisoned(&self.latest).clone();
        latest.map(|(tree, scale)| to_tree_update(&tree, scale))
    }
}

struct PlatformActionHandler {
    pending: PendingRequests,
    wake: WakeWindowLoop,
}

impl accesskit::ActionHandler for PlatformActionHandler {
    fn do_action(&mut self, request: accesskit::ActionRequest) {
        let Some(request) = AccessRequest::from_action(&request) else { return };
        lock_unpoisoned(&self.pending).push_back(request);
        (self.wake)();
    }
}

struct PlatformDeactivationHandler;

impl accesskit::DeactivationHandler for PlatformDeactivationHandler {
    fn deactivate_accessibility(&mut self) {
        // The latest tree is small, bounded by the visible scene and useful if
        // accessibility is activated again. There is no platform resource in
        // our shared state that needs releasing here.
    }
}

/// The platform accessibility adapter owned by the host window.
///
/// AccessKit stays entirely behind this type. Platform callbacks may arrive on
/// any thread, so actions enter a small queue and wake the winit event loop;
/// application state is only touched later, on the window thread.
pub struct PlatformAdapter {
    inner: accesskit_winit::Adapter,
    latest: LatestTree,
    pending: PendingRequests,
}

impl PlatformAdapter {
    /// Create the adapter before `window` is visible for the first time.
    pub fn new(
        event_loop: &ActiveEventLoop,
        window: &Window,
        wake: impl Fn() + Send + Sync + 'static,
    ) -> Self {
        let latest = Arc::new(Mutex::new(None));
        let pending = Arc::new(Mutex::new(VecDeque::new()));
        let wake: WakeWindowLoop = Arc::new(wake);

        let inner = accesskit_winit::Adapter::with_direct_handlers(
            event_loop,
            window,
            PlatformActivationHandler { latest: Arc::clone(&latest) },
            PlatformActionHandler { pending: Arc::clone(&pending), wake },
            PlatformDeactivationHandler,
        );

        Self { inner, latest, pending }
    }

    /// Forward every winit window event before the application handles it.
    pub fn process_event(&mut self, window: &Window, event: &WindowEvent) {
        self.inner.process_event(window, event);
    }

    /// Publish the semantic tree built alongside the latest visual scene.
    pub fn update(&mut self, tree: &AccessTree, scale: f32) {
        *lock_unpoisoned(&self.latest) = Some((tree.clone(), scale));
        self.inner.update_if_active(|| to_tree_update(tree, scale));
    }

    /// Drain requests on the window thread, preserving platform order.
    pub fn take_requests(&self) -> Vec<AccessRequest> {
        lock_unpoisoned(&self.pending).drain(..).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::a11y::AccessNode;

    fn sample_tree() -> AccessTree {
        let mut tree = AccessTree::new();
        tree.push(AccessNode::new(
            NodeId::new(1, 0),
            Role::Button,
            "Send",
            Rect::new(10.0, 20.0, 30.0, 40.0),
        ));
        tree.push(
            AccessNode::new(
                NodeId::new(1, 1),
                Role::ProgressIndicator,
                "Zero AI is making changes",
                Rect::new(0.0, 100.0, 200.0, 4.0),
            )
            .valued(0.6)
            .busy(true)
            .focusable(false),
        );
        tree.push(
            AccessNode::new(
                NodeId::new(1, 2),
                Role::Button,
                "Disabled thing",
                Rect::new(0.0, 200.0, 50.0, 20.0),
            )
            .disabled(true),
        );
        tree
    }

    #[test]
    fn every_declared_node_reaches_the_platform() {
        let tree = sample_tree();
        let update = to_tree_update(&tree, 1.0);
        // One per node, plus the root.
        assert_eq!(update.nodes.len(), tree.len() + 1);
    }

    #[test]
    fn the_root_parents_everything_in_declaration_order() {
        let tree = sample_tree();
        let update = to_tree_update(&tree, 1.0);
        let (id, root) = &update.nodes[0];
        assert_eq!(*id, PLATFORM_ROOT);
        assert_eq!(
            root.children(),
            tree.nodes().iter().map(|n| to_accesskit_id(n.id)).collect::<Vec<_>>()
        );
    }

    #[test]
    fn bounds_are_converted_to_physical_pixels() {
        let tree = sample_tree();
        let update = to_tree_update(&tree, 2.0);
        let (_, node) = update
            .nodes
            .iter()
            .find(|(id, _)| *id == accesskit::NodeId(NodeId::new(1, 0).0))
            .unwrap();
        let bounds = node.bounds().expect("bounds should be set");
        assert_eq!(bounds.x0, 20.0);
        assert_eq!(bounds.y0, 40.0);
        assert_eq!(bounds.x1, 80.0, "x0 + width, both scaled");
        assert_eq!(bounds.y1, 120.0);
    }

    #[test]
    fn a_scale_of_zero_does_not_collapse_every_element_onto_a_point() {
        let tree = sample_tree();
        let update = to_tree_update(&tree, 0.0);
        let (_, node) = &update.nodes[1];
        let bounds = node.bounds().unwrap();
        assert!(bounds.x1 > bounds.x0, "geometry must stay non-degenerate");
    }

    #[test]
    fn progress_carries_a_range_not_just_a_number() {
        let tree = sample_tree();
        let update = to_tree_update(&tree, 1.0);
        let (_, node) = update
            .nodes
            .iter()
            .find(|(id, _)| *id == accesskit::NodeId(NodeId::new(1, 1).0))
            .unwrap();

        assert_eq!(node.numeric_value(), Some(60.0), "f32 noise must not reach the announcement");
        assert_eq!(node.min_numeric_value(), Some(0.0));
        assert_eq!(node.max_numeric_value(), Some(100.0), "a value with no range is meaningless");
        assert!(node.is_busy());
    }

    #[test]
    fn focus_is_only_advertised_where_it_can_actually_land() {
        let tree = sample_tree();
        let update = to_tree_update(&tree, 1.0);

        let focusable = |id: NodeId| {
            let (_, node) =
                update.nodes.iter().find(|(nid, _)| *nid == accesskit::NodeId(id.0)).unwrap();
            node.supports_action(accesskit::Action::Focus)
        };

        assert!(focusable(NodeId::new(1, 0)), "an enabled button should offer focus");
        assert!(!focusable(NodeId::new(1, 1)), "a progress bar is not a focus target");
        assert!(!focusable(NodeId::new(1, 2)), "a disabled control must not be offered");
    }

    #[test]
    fn the_focused_element_is_named_in_the_update() {
        let mut tree = sample_tree();
        tree.focus(NodeId::new(1, 0));
        let update = to_tree_update(&tree, 1.0);
        assert_eq!(update.focus, accesskit::NodeId(NodeId::new(1, 0).0));
    }

    #[test]
    fn with_nothing_focused_the_platform_is_pointed_at_the_root() {
        // Naming a node that is not in the update leaves the platform pointing
        // at nothing, which some screen readers handle badly.
        let tree = sample_tree();
        let update = to_tree_update(&tree, 1.0);
        assert_eq!(update.focus, PLATFORM_ROOT);
        assert!(update.nodes.iter().any(|(id, _)| *id == update.focus));
    }

    #[test]
    fn every_role_maps_to_something_the_platform_understands() {
        // A role that fell through to a generic container would lose meaning
        // silently, so every one is mapped explicitly.
        for role in [
            Role::Window,
            Role::Group,
            Role::Label,
            Role::Button,
            Role::Toggle,
            Role::Tab,
            Role::TabList,
            Role::List,
            Role::ListItem,
            Role::TextInput,
            Role::ProgressIndicator,
            Role::ScrollArea,
            Role::Article,
        ] {
            let mapped = to_accesskit_role(role);
            assert_ne!(
                mapped,
                accesskit::Role::GenericContainer,
                "{role:?} lost its meaning in translation"
            );
        }
    }

    #[test]
    fn state_survives_the_translation() {
        let mut tree = AccessTree::new();
        tree.push(
            AccessNode::new(
                NodeId::new(2, 0),
                Role::Button,
                "Context",
                Rect::new(0.0, 0.0, 100.0, 20.0),
            )
            .expanded(false)
            .described("9 groups"),
        );

        let update = to_tree_update(&tree, 1.0);
        let (_, node) = &update.nodes[1];
        assert_eq!(node.label(), Some("Context"));
        assert_eq!(node.description(), Some("9 groups"));
        assert_eq!(node.is_expanded(), Some(false));
    }

    #[test]
    fn platform_requests_translate_into_our_vocabulary() {
        let request = accesskit::ActionRequest {
            action: accesskit::Action::Click,
            target_tree: accesskit::TreeId::ROOT,
            target_node: accesskit::NodeId(NodeId::new(1, 0).0),
            data: None,
        };
        assert_eq!(
            AccessRequest::from_action(&request),
            Some(AccessRequest::Activate(NodeId::new(1, 0)))
        );
        assert_eq!(AccessRequest::from_action(&request).unwrap().target(), NodeId::new(1, 0));
    }

    #[test]
    fn an_action_we_do_not_handle_is_declined_rather_than_guessed_at() {
        let request = accesskit::ActionRequest {
            action: accesskit::Action::Increment,
            target_tree: accesskit::TreeId::ROOT,
            target_node: accesskit::NodeId(1),
            data: None,
        };
        assert_eq!(AccessRequest::from_action(&request), None);
    }

    #[test]
    fn an_empty_tree_still_produces_a_valid_update() {
        let update = to_tree_update(&AccessTree::new(), 1.0);
        assert_eq!(update.nodes.len(), 1, "the root alone");
        assert_eq!(update.focus, PLATFORM_ROOT);
    }

    #[test]
    fn a_supported_platform_action_is_queued_and_wakes_the_window_loop() {
        use accesskit::ActionHandler as _;
        use std::collections::VecDeque;
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::{Arc, Mutex};

        let pending = Arc::new(Mutex::new(VecDeque::new()));
        let wakes = Arc::new(AtomicUsize::new(0));
        let wakes_for_handler = Arc::clone(&wakes);
        let mut handler = PlatformActionHandler {
            pending: Arc::clone(&pending),
            wake: Arc::new(move || {
                wakes_for_handler.fetch_add(1, Ordering::SeqCst);
            }),
        };

        handler.do_action(accesskit::ActionRequest {
            action: accesskit::Action::Focus,
            target_tree: accesskit::TreeId::ROOT,
            target_node: accesskit::NodeId(NodeId::new(4, 2).0),
            data: None,
        });

        assert_eq!(
            pending.lock().unwrap().pop_front(),
            Some(AccessRequest::Focus(NodeId::new(4, 2)))
        );
        assert_eq!(wakes.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn an_unsupported_platform_action_neither_queues_nor_wakes() {
        use accesskit::ActionHandler as _;
        use std::collections::VecDeque;
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::{Arc, Mutex};

        let pending = Arc::new(Mutex::new(VecDeque::new()));
        let wakes = Arc::new(AtomicUsize::new(0));
        let wakes_for_handler = Arc::clone(&wakes);
        let mut handler = PlatformActionHandler {
            pending: Arc::clone(&pending),
            wake: Arc::new(move || {
                wakes_for_handler.fetch_add(1, Ordering::SeqCst);
            }),
        };

        handler.do_action(accesskit::ActionRequest {
            action: accesskit::Action::Increment,
            target_tree: accesskit::TreeId::ROOT,
            target_node: accesskit::NodeId(NodeId::new(4, 2).0),
            data: None,
        });

        assert!(pending.lock().unwrap().is_empty());
        assert_eq!(wakes.load(Ordering::SeqCst), 0);
    }
}
