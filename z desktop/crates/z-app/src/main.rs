//! Z Desktop — application entry point.
//!
//! Wires the four layers together and hands control to ZeroGPUI:
//!
//! ```text
//! z-protocol  contracts        (commands in, events out)
//! z-core      Agent Runtime    (threads, turns, tools, providers)
//! z-shell     workspace model  (what exists, where, and what is inside it)
//! z-app       view             (turns that model into a scene)
//! z-gpui      runtime          (owns the window and draws the scene)
//! ```

mod content;
mod view;

use std::collections::HashMap;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};
use view::WorkspaceView;
use winit::keyboard::{Key, ModifiersState, NamedKey};
use z_core::runtime::Runtime;
use z_gpui::{
    AccessRequest, BackendInfo, FrameStats, FrameTiming, Point, Rect, Scene, SceneSource,
    TimingSummary, WindowConfig,
};
use z_gpui::window::HostEvent;
use z_protocol::{Command, Event};
use z_shell::Preset;

/// Events waiting to be projected into the view. Drained at the top of every
/// frame so the scene never renders stale runtime state.
type EventQueue = Arc<Mutex<Vec<Event>>>;

struct App {
    view: WorkspaceView,
    /// Cycled with the arrow keys so a reviewer can see every preset without a
    /// settings surface existing yet.
    preset_index: usize,
    /// Last viewport seen. Focus traversal needs a size to resolve the tree
    /// against, and key events arrive outside the frame loop.
    viewport: Rect,
    /// The one conversation this build runs. Multi-thread comes later; one
    /// honest thread beats several fake ones.
    thread_id: String,
    command_tx: Sender<(u64, Command)>,
    next_command: u64,
    events: EventQueue,
    /// A turn is producing output right now (drives Escape semantics).
    streaming: bool,
    /// call_id → tool name for steps started but not yet finished.
    step_tools: HashMap<String, String>,
    /// The call awaiting approval, shared with the resolve callback.
    pending_call: Arc<Mutex<Option<String>>>,
    /// Runtime events, held until the window hands over its wake proxy.
    event_rx: Option<Receiver<Event>>,
}

impl App {
    fn new() -> Self {
        let (command_tx, command_rx): (Sender<(u64, Command)>, Receiver<(u64, Command)>) =
            channel();
        let (event_tx, event_rx): (Sender<Event>, Receiver<Event>) = channel();

        // The Agent Runtime owns its own thread; the UI never blocks on it and
        // the runtime never blocks on the UI.
        std::thread::Builder::new()
            .name("z-runtime".into())
            .spawn(move || Runtime::new(event_tx, command_rx).serve())
            .expect("could not spawn the Agent Runtime");

        let events: EventQueue = Arc::new(Mutex::new(Vec::new()));
        let pending_call: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));

        let mut app = Self {
            view: WorkspaceView::new(),
            preset_index: 0,
            viewport: Rect::new(0.0, 0.0, 1440.0, 810.0),
            thread_id: z_core::new_id("thread"),
            command_tx,
            next_command: 1,
            events,
            streaming: false,
            step_tools: HashMap::new(),
            pending_call,
            event_rx: Some(event_rx),
        };

        // Composer send → Agent Runtime.
        let tx = app.command_tx.clone();
        let thread_id = app.thread_id.clone();
        app.view.on_send = Some(Box::new(move |text| {
            static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
            let id = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let _ = tx.send((id, Command::SendMessage { thread_id: thread_id.clone(), text }));
        }));

        // Thread selection → Agent Runtime (completes the SwitchThread loop).
        let tx = app.command_tx.clone();
        app.view.on_switch_thread = Some(Box::new(move |thread_id| {
            static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
            let id = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let _ = tx.send((id, Command::SwitchThread { thread_id }));
        }));

        // Approval decision → Agent Runtime.
        let tx = app.command_tx.clone();
        let pending = Arc::clone(&app.pending_call);
        app.view.on_resolve = Some(Box::new(move |approved| {
            if let Some(call_id) = pending.lock().unwrap().take() {
                let _ = tx.send((
                    approved as u64,
                    Command::ResolveApproval { call_id, approved },
                ));
            }
        }));

        app
    }

    fn send(&mut self, command: Command) {
        let id = self.next_command;
        self.next_command += 1;
        let _ = self.command_tx.send((id, command));
    }

    /// Apply queued runtime events to the view's projection.
    fn drain_events(&mut self) -> bool {
        let batch: Vec<Event> = std::mem::take(&mut *self.events.lock().unwrap());
        if batch.is_empty() {
            return false;
        }
        let mut changed = false;
        for event in batch {
            changed |= self.apply_event(event);
        }
        if changed {
            self.view.scroll_chat_to_end();
        }
        changed
    }

    fn apply_event(&mut self, event: Event) -> bool {
        match event {
            Event::Accepted { .. } => false,
            Event::ThreadList { threads } => {
                // core-021: mirror the snapshot into the sidebar's thread rows.
                self.view.threads = threads;
                true
            }
            // core-024 companion: mirror the runtime-confirmed active thread.
            Event::ThreadSwitched { thread_id } => {
                self.view.active_thread_id = Some(thread_id);
                true
            }
            Event::EvidenceSummary { items } => {
                // ui-040: dedupe to one badge per (kind, ok) — a badge answers
                // "did build/tests pass", not how many runs happened.
                let mut badges: Vec<(String, bool)> = Vec::new();
                for item in items {
                    let badge = (item.kind, item.ok);
                    if !badges.contains(&badge) {
                        badges.push(badge);
                    }
                }
                self.view.evidence_badges = badges;
                true
            }
            // sup-017/024: an appealed verdict was journaled as overridden;
            // surface it so the user sees the appeal took effect.
            Event::VerdictOverridden { turn_id, blocked_cleared } => {
                self.view.status_line = format!(
                    "verdict overridden for {turn_id} ({})",
                    if blocked_cleared { "cleared" } else { "not persisted" }
                );
                true
            }
            Event::SteeringQueued { depth, .. } => {
                self.view.steering_depth = depth as u32;

                self.view.status_line = if depth > 0 {
                    format!("steering queued ({depth} pending)")
                } else {
                    "ready".into()
                };
                true
            }
            Event::ProviderStatus { ok, message } => {
                self.view.status_line = if ok {
                    message
                } else {
                    format!("provider problem: {message}")
                };
                true
            }
            // set-004: mirror accepted setting changes into the status line.
            Event::SettingChanged { key, value } => {
                self.view.status_line = format!("settings: {key} = {value}");
                true
            }
            Event::ProjectIndexed { path, files, symbols } => {
                let name = std::path::Path::new(&path)
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or(&path)
                    .to_string();
                self.view.conversation.project = name;
                self.view.conversation.entries = vec![
                    content::ContextEntry { label: "Files", count: files as u32 },
                    content::ContextEntry { label: "Symbols", count: symbols as u32 },
                ];
                self.view.status_line = format!("indexed {path}: {files} files, {symbols} symbols");
                // ui-030: mirror the fresh index into the sidebar rows.
                self.view.sidebar_items = vec![
                    ("Files".into(), format!("{files} indexed")),
                    ("Symbols".into(), format!("{symbols} indexed")),
                ];
                true
            }
            Event::TextDone { .. } => false,
            Event::TurnStarted { .. } => {
                self.streaming = true;
                self.view.conversation.ensure_streaming_agent();
                true
            }
            Event::TextDelta { delta, .. } => {
                self.view.conversation.ensure_streaming_agent().body.push_str(&delta);
                true
            }
            Event::StepStarted { call_id, tool, detail, .. } => {
                self.step_tools.insert(call_id, tool.clone());
                let message = self.view.conversation.ensure_streaming_agent();
                message.plan.push(content::PlanStep {
                    title: format!("{tool} · {detail}"),
                    state: content::StepState::InProgress,
                });
                Self::refresh_progress(message);
                true
            }
            Event::StepFinished { call_id, ok, summary, .. } => {
                let tool = self.step_tools.remove(&call_id).unwrap_or_default();
                self.view.conversation.finish_step(&tool, ok, &summary);
                if let Some(message) =
                    self.view.conversation.messages.last_mut().filter(|m| m.role == content::Role::Agent)
                {
                    Self::refresh_progress(message);
                }
                true
            }
            Event::ApprovalRequested { call_id, tool, detail, risk, .. } => {
                *self.pending_call.lock().unwrap() = Some(call_id);
                self.view.pending_approval = Some(view::PendingApproval {
                    call_id: String::new(), // routing goes through pending_call
                    tool,
                    detail: format!("{detail} ({risk:?})"),
                });
                true
            }
            Event::TurnFinished { ok, error, .. } => {
                self.streaming = false;
                self.step_tools.clear();
                *self.pending_call.lock().unwrap() = None;
                self.view.pending_approval = None;
                // An empty streamed body with an error means nothing was said;
                // surface the failure rather than leaving a blank bubble.
                if let Some(error) = error {
                    self.view.status_line = format!("turn failed: {error}");
                    if let Some(message) = self
                        .view
                        .conversation
                        .messages
                        .last_mut()
                        .filter(|m| m.role == content::Role::Agent)
                        .filter(|m| m.body.trim().is_empty() && m.plan.is_empty())
                    {
                        message.body = format!("⚠ {error}");
                    }
                } else {
                    self.view.status_line = if ok { "ready".into() } else { "stopped".into() };
                }
                true
            }
        }
    }

    /// Recompute the work-in-progress counters from the actual step states, so
    /// the progress block can never disagree with the checklist above it.
    fn refresh_progress(message: &mut content::Message) {
        if message.plan.is_empty() {
            message.progress = None;
            return;
        }
        let total = message.plan.len() as u32;
        let done = message
            .plan
            .iter()
            .filter(|s| matches!(s.state, content::StepState::Completed))
            .count() as u32;
        let detail = message
            .plan
            .iter()
            .rev()
            .find(|s| s.state == content::StepState::InProgress)
            .map(|s| s.title.clone())
            .unwrap_or_default();
        let running = message
            .plan
            .iter()
            .any(|s| s.state == content::StepState::InProgress);
        message.progress = Some(content::Progress {
            title: if running { "Z is working".into() } else { "Z finished this pass".into() },
            detail,
            done,
            total,
        });
    }

    fn type_character(&mut self, c: char) -> bool {
        self.view.input.push(c);
        true
    }

    fn send_composer(&mut self) -> bool {
        if self.view.input.trim().is_empty() {
            return false;
        }
        let text = std::mem::take(&mut self.view.input);
        let id = self.next_command;
        self.next_command += 1;
        // A running turn is steered, not interrupted: queued text is injected
        // between tool rounds instead of starting a second concurrent turn.
        let command = if self.streaming {
            Command::EnqueueMessage {
                thread_id: self.thread_id.clone(),
                text,
            }
        } else {
            Command::SendMessage {
                thread_id: self.thread_id.clone(),
                text,
            }
        };
        let _ = self.command_tx.send((id, command));
        true
    }

    fn step_preset(&mut self, forward: bool) {
        let count = Preset::BUILT_IN.len();
        self.preset_index = if forward {
            (self.preset_index + 1) % count
        } else {
            (self.preset_index + count - 1) % count
        };
        let preset = Preset::BUILT_IN[self.preset_index];
        self.view.workspace.apply_preset(preset);
        log::info!("layout preset: {}", preset.label());
    }
}

impl SceneSource for App {
    fn build(&mut self, viewport: Rect, _scale: f32) -> Scene {
        self.viewport = viewport;
        self.drain_events();
        self.view.build(viewport)
    }

    fn clear_color(&self) -> z_tokens::Rgba {
        self.view.theme.colors.canvas
    }

    fn on_ready(&mut self, proxy: winit::event_loop::EventLoopProxy<HostEvent>) {
        // Forward runtime events into the loop: each one queues here and wakes
        // the window, so streaming text appears without polling.
        let Some(event_rx) = self.event_rx.take() else { return };
        let queue = Arc::clone(&self.events);
        std::thread::Builder::new()
            .name("z-events".into())
            .spawn(move || {
                for event in event_rx {
                    queue.lock().unwrap().push(event);
                    let _ = proxy.send_event(HostEvent::Wake);
                }
            })
            .expect("could not spawn the event pump");
    }

    fn on_wake(&mut self) -> bool {
        self.drain_events()
    }

    fn on_key(&mut self, key: &Key, modifiers: ModifiersState) -> bool {
        match key {
            Key::Character(text) => {
                // Plain typing goes to the composer; Ctrl/Alt chords stay free
                // for shortcuts that do not exist yet.
                if modifiers.control_key() || modifiers.alt_key() || modifiers.super_key() {
                    return false;
                }
                text.chars().fold(false, |_, c| self.type_character(c) || true)
            }
            Key::Named(NamedKey::Backspace) => {
                self.view.input.pop();
                true
            }
            Key::Named(NamedKey::Enter) => {
                if modifiers.shift_key() || self.view.input.trim().is_empty() {
                    self.view.activate_focused(self.viewport)
                } else {
                    self.send_composer()
                }
            }
            Key::Named(NamedKey::Escape) => {
                if self.streaming {
                    self.send(Command::CancelTurn { thread_id: self.thread_id.clone() });
                    // Optimistically leave the streaming state: the runtime
                    // confirms with TurnFinished, but the UI must not keep
                    // claiming work is running after the user cancelled.
                    self.streaming = false;
                    self.view.status_line = "cancelling…".into();
                    true
                } else if self.view.pending_approval.is_some() {
                    self.view.pending_approval = None;
                    *self.pending_call.lock().unwrap() = None;
                    true
                } else {
                    false
                }
            }
            // Tab walks the interface in the order it is meant to be read.
            Key::Named(NamedKey::Tab) => {
                let moved = self.view.move_focus(!modifiers.shift_key(), self.viewport);
                if let Some(id) = self.view.focused() {
                    log::debug!("focus: {id:?}");
                }
                moved
            }
            // A keyboard activation and an assistive-technology activation
            // deliberately share `activate_focused`, so they cannot drift into
            // two subtly different command paths.
            Key::Named(NamedKey::Space) => self.view.activate_focused(self.viewport),
            Key::Named(NamedKey::ArrowRight) => {
                if !self.view.move_focus_in_tab_strip(true, self.viewport) {
                    self.step_preset(true);
                }
                true
            }
            Key::Named(NamedKey::ArrowLeft) => {
                if !self.view.move_focus_in_tab_strip(false, self.viewport) {
                    self.step_preset(false);
                }
                true
            }
            Key::Named(NamedKey::ArrowDown) => self.view.move_focus_in_list(true, self.viewport),
            Key::Named(NamedKey::ArrowUp) => self.view.move_focus_in_list(false, self.viewport),
            Key::Named(NamedKey::End) => {
                self.view.scroll_chat_to_end();
                true
            }
            Key::Named(NamedKey::PageDown) => {
                self.view.scroll_chat_by(400.0);
                true
            }
            Key::Named(NamedKey::PageUp) => {
                self.view.scroll_chat_by(-400.0);
                true
            }
            _ => false,
        }
    }

    fn on_scroll(&mut self, delta: f32) -> bool {
        self.view.scroll_chat_by(delta);
        true
    }

    fn on_click(&mut self, position: Point) -> bool {
        self.view.click(position, self.viewport)
    }

    fn on_access_request(&mut self, request: AccessRequest) -> bool {
        match request {
            AccessRequest::Focus(id) => self.view.focus(id, self.viewport),
            AccessRequest::Activate(id) => {
                let focus_changed = self.view.focus(id, self.viewport);
                if focus_changed || self.view.focused() == Some(id) {
                    self.view.activate_focused(self.viewport) || focus_changed
                } else {
                    false
                }
            }
            AccessRequest::ScrollIntoView(id) => self.view.scroll_access_node_into_view(id),
        }
    }

    fn on_backend_ready(&mut self, info: &BackendInfo) {
        log::info!("ZeroRender: {} on {} ({})", info.backend, info.adapter, info.device_type);
    }

    fn on_frame(&mut self, stats: FrameStats, timing: FrameTiming) {
        if stats.skipped {
            return;
        }
        let (stage, spent) = timing.slowest();
        log::debug!(
            "frame: {:.2}ms · {} quads, {} glyphs, {} draw calls · slowest {} {:.2}ms",
            timing.total.as_secs_f64() * 1000.0,
            stats.quads,
            stats.glyphs,
            stats.draw_calls,
            stage.name(),
            spent.as_secs_f64() * 1000.0
        );
    }

    fn on_session_end(&mut self, summary: TimingSummary) {
        println!("{summary}");
    }
}

/// Mutates a view into one of the non-preset states worth reviewing.
type VariantSetup = fn(&mut WorkspaceView);

/// Render the default view plus every built-in preset to PNG.
fn capture_presets(dir: &str) -> Result<(), String> {
    use z_gpui::OffscreenRenderer;

    std::fs::create_dir_all(dir).map_err(|e| format!("could not create {dir}: {e}"))?;

    let scale = 2.0;
    let viewport = Rect::new(0.0, 0.0, 1536.0, 960.0);
    let mut renderer = OffscreenRenderer::new(scale)?;
    println!(
        "ZeroRender: {} on {} ({})",
        renderer.backend().backend,
        renderer.backend().adapter,
        renderer.backend().device_type
    );

    for preset in Preset::BUILT_IN {
        let mut view = WorkspaceView::new();
        view.workspace.apply_preset(*preset);
        let name = preset.label().to_lowercase().replace(' ', "-");

        let _ = view.build(viewport);
        let scene = view.build(viewport);
        let capture = renderer.capture(&scene, viewport, view.theme.colors.canvas)?;
        let path = std::path::Path::new(dir).join(format!("workspace-{name}.png"));
        capture.write_png(&path)?;
        println!(
            "{:<24} {}x{}  {} quads, {} glyphs, {} draw calls",
            path.display(),
            capture.width,
            capture.height,
            capture.stats.quads,
            capture.stats.glyphs,
            capture.stats.draw_calls
        );
    }

    let variants: [(&str, VariantSetup); 4] = [
        ("floating-tool-open", |view| view.workspace.view.floating_tool_open = true),
        ("plan-failed", |view| view.set_conversation(content::Conversation::with_failed_step())),
        ("long-thread", |view| {
            view.set_conversation(content::Conversation::long(10_000));
            view.scroll_chat_to_end();
        }),
        ("focus-ring", |view| {
            let viewport = Rect::new(0.0, 0.0, 1536.0, 960.0);
            for _ in 0..8 {
                view.move_focus(true, viewport);
            }
        }),
    ];

    for (name, configure) in variants {
        let mut view = WorkspaceView::new();
        configure(&mut view);

        let _ = view.build(viewport);
        let scene = view.build(viewport);

        let capture = renderer.capture(&scene, viewport, view.theme.colors.canvas)?;
        let path = std::path::Path::new(dir).join(format!("workspace-{name}.png"));
        capture.write_png(&path)?;
        println!(
            "{:<24} {}x{}  {} quads, {} glyphs",
            path.display(),
            capture.width,
            capture.height,
            capture.stats.quads,
            capture.stats.glyphs
        );
    }

    Ok(())
}

fn main() {
    env_logger::Builder::from_env(
        env_logger::Env::default().default_filter_or("z_app=info,z_core=info,z_gpui=warn"),
    )
    .init();

    let args: Vec<String> = std::env::args().skip(1).collect();

    if args.iter().any(|a| a == "--check") {
        let mut view = WorkspaceView::new();
        let viewport = Rect::new(0.0, 0.0, 1536.0, 1024.0);
        let scene = view.build(viewport);
        println!(
            "scene ok: {} quads, {} text runs, {} panels collapsed",
            scene.quad_count(),
            scene.text_count(),
            view.collapsed_panels(viewport).len()
        );
        return;
    }

    if let Some(index) = args.iter().position(|a| a == "--long") {
        // Headless virtualization demo; not wired to the runtime.
        let count: usize = args.get(index + 1).and_then(|n| n.parse().ok()).unwrap_or(10_000);
        let mut view = WorkspaceView::new();
        view.set_conversation(content::Conversation::long(count));
        view.scroll_chat_to_end();
        log::info!("seeded a {count}-message thread");
        return;
    }

    if let Some(index) = args.iter().position(|a| a == "--shot") {
        let dir = args.get(index + 1).cloned().unwrap_or_else(|| ".".into());
        if let Err(error) = capture_presets(&dir) {
            eprintln!("{error}");
            std::process::exit(1);
        }
        return;
    }

    let config = WindowConfig {
        title: "Z Desktop".into(),
        single_frame: args.iter().any(|a| a == "--single-frame"),
        ..WindowConfig::default()
    };

    let mut app = App::new();

    // Restore the BYOK provider configuration saved by a previous session.
    let data_dir = z_core::runtime::data_dir();
    if let Ok(json) = std::fs::read_to_string(data_dir.join("config.json")) {
        if let Ok(provider_config) = serde_json::from_str::<z_protocol::ProviderConfig>(&json) {
            app.send(Command::ConfigureProvider { config: provider_config });
            log::info!("restored provider configuration");
        }
    }

    // `--project <path>` opens and indexes a repository at launch.
    if let Some(index) = args.iter().position(|a| a == "--project") {
        if let Some(path) = args.get(index + 1) {
            app.send(Command::OpenProject { path: path.clone() });
        }
    }

    if let Err(error) = z_gpui::run(config, app) {
        log::error!("{error}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod access_tests {
    use super::*;
    use z_gpui::{AccessRequest, NodeId};
    use z_shell::ContextSection;

    fn node_id(app: &mut App, label: &str) -> NodeId {
        let viewport = app.viewport;
        app.build(viewport, 1.0)
            .access()
            .nodes()
            .iter()
            .find(|node| node.label == label)
            .map(|node| node.id)
            .unwrap_or_else(|| panic!("{label:?} is missing from the semantic tree"))
    }

    fn node_bounds(app: &mut App, label: &str) -> Rect {
        let viewport = app.viewport;
        app.build(viewport, 1.0)
            .access()
            .nodes()
            .iter()
            .find(|node| node.label == label)
            .map(|node| node.bounds)
            .unwrap_or_else(|| panic!("{label:?} is missing from the semantic tree"))
    }

    #[test]
    fn a_platform_focus_request_reaches_the_view() {
        let mut app = App::new();
        let scene = app.build(app.viewport, 1.0);
        let target = scene
            .access()
            .nodes()
            .iter()
            .find(|node| node.label == "Chat · Auth Flow")
            .map(|node| node.id)
            .expect("the reference chat tab should exist");

        assert!(app.on_access_request(AccessRequest::Focus(target)));
        assert_eq!(app.view.focused(), Some(target));
    }

    #[test]
    fn a_platform_activation_request_uses_the_same_tab_command_as_the_ui() {
        let mut app = App::new();
        let scene = app.build(app.viewport, 1.0);
        let target = scene
            .access()
            .nodes()
            .iter()
            .find(|node| node.label == "IDE")
            .map(|node| node.id)
            .expect("the IDE tab should exist");

        assert!(app.on_access_request(AccessRequest::Activate(target)));
        assert_eq!(
            app.view.workspace.view.tabs.active_tab().map(|tab| tab.kind.label()),
            Some("IDE".to_string())
        );

        assert!(!app.on_access_request(AccessRequest::Focus(NodeId(u64::MAX - 1))));
    }

    #[test]
    fn enter_activates_the_focused_tab() {
        let mut app = App::new();
        let viewport = app.viewport;
        let ide = node_id(&mut app, "IDE");

        assert!(app.view.focus(ide, viewport));
        assert!(app.on_key(&Key::Named(NamedKey::Enter), ModifiersState::empty()));
        assert_eq!(
            app.view.workspace.view.tabs.active_tab().map(|tab| tab.kind.label()),
            Some("IDE".to_string())
        );
    }

    #[test]
    fn space_activates_the_focused_context_control() {
        let mut app = App::new();
        let viewport = app.viewport;
        let project = node_id(&mut app, "Project");

        assert!(app.view.workspace.view.is_expanded(ContextSection::Project));
        assert!(app.view.focus(project, viewport));
        assert!(app.on_key(&Key::Named(NamedKey::Space), ModifiersState::empty()));
        assert!(!app.view.workspace.view.is_expanded(ContextSection::Project));
    }

    #[test]
    fn mouse_click_on_a_tab_uses_the_same_command_path() {
        let mut app = App::new();
        let ide = node_id(&mut app, "IDE");
        let bounds = node_bounds(&mut app, "IDE");

        assert!(app.on_click(bounds.center()));
        assert_eq!(app.view.focused(), Some(ide));
        assert_eq!(
            app.view.workspace.view.tabs.active_tab().map(|tab| tab.kind.label()),
            Some("IDE".to_string())
        );
    }

    #[test]
    fn mouse_click_on_a_context_control_toggles_it() {
        let mut app = App::new();
        let project = node_id(&mut app, "Project");
        let bounds = node_bounds(&mut app, "Project");

        assert!(app.view.workspace.view.is_expanded(ContextSection::Project));
        assert!(app.on_click(bounds.center()));
        assert_eq!(app.view.focused(), Some(project));
        assert!(!app.view.workspace.view.is_expanded(ContextSection::Project));
    }

    #[test]
    fn horizontal_arrows_roam_the_focused_tab_strip_before_changing_a_preset() {
        let mut app = App::new();
        let viewport = app.viewport;
        let chat = node_id(&mut app, "Chat · Auth Flow");
        let ide = node_id(&mut app, "IDE");

        assert!(app.view.focus(chat, viewport));
        let preset_before = app.preset_index;
        assert!(app.on_key(&Key::Named(NamedKey::ArrowRight), ModifiersState::empty()));
        assert_eq!(app.view.focused(), Some(ide));
        assert_eq!(app.preset_index, preset_before, "tab navigation must not change layout");

        assert!(app.on_key(&Key::Named(NamedKey::ArrowLeft), ModifiersState::empty()));
        assert_eq!(app.view.focused(), Some(chat));
    }

    #[test]
    fn vertical_arrows_roam_the_focused_navigation_list() {
        let mut app = App::new();
        let viewport = app.viewport;
        let home = node_id(&mut app, "Home");
        let projects = node_id(&mut app, "Projects");

        assert!(app.view.focus(home, viewport));
        assert!(app.on_key(&Key::Named(NamedKey::ArrowDown), ModifiersState::empty()));
        assert_eq!(app.view.focused(), Some(projects));

        assert!(app.on_key(&Key::Named(NamedKey::ArrowUp), ModifiersState::empty()));
        assert_eq!(app.view.focused(), Some(home));
    }

    #[test]
    fn typed_characters_reach_the_composer_and_enter_sends_them() {
        let mut app = App::new();
        let viewport = app.viewport;

        for c in "hello".chars() {
            assert!(app.on_key(&Key::Character(c.to_string().into()), ModifiersState::empty()));
        }
        assert_eq!(app.view.input, "hello");

        assert!(app.on_key(&Key::Named(NamedKey::Backspace), ModifiersState::empty()));
        assert_eq!(app.view.input, "hell");

        assert!(app.on_key(&Key::Named(NamedKey::Enter), ModifiersState::empty()));
        assert!(app.view.input.is_empty(), "sending must clear the composer");
    }

    #[test]
    fn enter_with_an_empty_composer_still_activates_the_focused_control() {
        let mut app = App::new();
        let viewport = app.viewport;
        let ide = node_id(&mut app, "IDE");

        assert!(app.view.focus(ide, viewport));
        assert!(app.on_key(&Key::Named(NamedKey::Enter), ModifiersState::empty()));
        assert_eq!(
            app.view.workspace.view.tabs.active_tab().map(|tab| tab.kind.label()),
            Some("IDE".to_string())
        );
    }

    #[test]
    fn escape_cancels_a_running_turn_before_it_quits() {
        let mut app = App::new();
        app.streaming = true;

        assert!(app.on_key(&Key::Named(NamedKey::Escape), ModifiersState::empty()));
        assert!(!app.streaming);
    }

    #[test]
    fn runtime_events_project_into_honest_view_state() {
        let mut app = App::new();
        let thread = app.thread_id.clone();

        app.events.lock().unwrap().push(Event::TextDelta {
            thread_id: thread.clone(),
            turn_id: "t".into(),
            delta: "Hello".into(),
        });
        // A delta before a turn started still lands in a streaming message.
        assert!(app.drain_events());
        assert_eq!(app.view.conversation.messages.len(), 1);

        app.events.lock().unwrap().push(Event::StepStarted {
            thread_id: thread.clone(),
            turn_id: "t".into(),
            call_id: "c1".into(),
            tool: "fs_read".into(),
            detail: "src/main.rs".into(),
        });
        app.events.lock().unwrap().push(Event::StepFinished {
            thread_id: thread.clone(),
            turn_id: "t".into(),
            call_id: "c1".into(),
            ok: true,
            summary: "12 lines".into(),
        });
        assert!(app.drain_events());

        let message = &app.view.conversation.messages[0];
        assert_eq!(message.plan.len(), 1);
        assert_eq!(message.plan[0].state, content::StepState::Completed);
        assert!(message.plan[0].title.contains("fs_read"));
    }

    #[test]
    fn a_message_sent_mid_turn_is_enqueued_as_steering_not_a_second_turn() {
        // The app-layer half of the steering proof: with a turn streaming,
        // the composer must route through EnqueueMessage so the text lands
        // between tool rounds instead of starting a concurrent turn.
        let mut app = App::new();
        let (cmd_tx, cmd_rx) = std::sync::mpsc::channel();
        app.command_tx = cmd_tx;
        app.streaming = true;

        for c in "slow down".chars() {
            assert!(app.on_key(&Key::Character(c.to_string().into()), ModifiersState::empty()));
        }
        assert!(app.send_composer());
        assert!(app.view.input.is_empty(), "sending must clear the composer");

        match cmd_rx.try_recv() {
            Ok((_, Command::EnqueueMessage { thread_id, text })) => {
                assert_eq!(thread_id, app.thread_id);
                assert_eq!(text, "slow down");
            }
            other => panic!("expected EnqueueMessage mid-turn, got {other:?}"),
        }

        // Idle: the same input routes as a normal SendMessage.
        app.streaming = false;
        for c in "start fresh".chars() {
            assert!(app.on_key(&Key::Character(c.to_string().into()), ModifiersState::empty()));
        }
        assert!(app.send_composer());
        match cmd_rx.try_recv() {
            Ok((_, Command::SendMessage { .. })) => {}
            other => panic!("expected SendMessage when idle, got {other:?}"),
        }
    }

    #[test]
    fn a_steering_queued_event_updates_the_depth_indicator() {
        let mut app = App::new();
        let thread = app.thread_id.clone();

        app.events.lock().unwrap().push(Event::SteeringQueued { thread_id: thread, depth: 2 });
        assert!(app.drain_events());
        assert_eq!(app.view.steering_depth, 2);
        assert!(app.view.status_line.contains("steering queued (2 pending)"));
    }

    #[test]
    fn a_thread_list_event_populates_the_sidebar_mirror_and_triggers_a_frame() {
        let mut app = App::new();

        app.events.lock().unwrap().push(Event::ThreadList {
            threads: vec![z_protocol::ThreadInfo {
                id: "t1".into(),
                title: "Refactor tokens".into(),
                message_count: 7,
                updated_ms: 42,
            }],
        });
        assert!(
            app.drain_events(),
            "a ThreadList event must trigger a re-render"
        );
        assert_eq!(app.view.threads.len(), 1);
        assert_eq!(app.view.threads[0].title, "Refactor tokens");
        assert_eq!(app.view.threads[0].message_count, 7);
    }

    #[test]
    fn an_evidence_summary_populates_deduped_badges_and_triggers_a_frame() {
        let mut app = App::new();

        app.events.lock().unwrap().push(Event::EvidenceSummary {
            items: vec![
                z_protocol::EvidenceInfo { id: "ev-1".into(), kind: "tests".into(), ok: true, summary: "5 passed".into() },
                z_protocol::EvidenceInfo { id: "ev-2".into(), kind: "build".into(), ok: false, summary: "exit code 1".into() },
                z_protocol::EvidenceInfo { id: "ev-3".into(), kind: "tests".into(), ok: true, summary: "duplicate run".into() },
            ],
        });
        assert!(
            app.drain_events(),
            "an EvidenceSummary event must trigger a re-render"
        );
        // Deduped by (kind, ok): the second green tests row collapses.
        assert_eq!(
            app.view.evidence_badges,
            vec![("tests".to_string(), true), ("build".to_string(), false)]
        );

        // And the badges reach the scene as words in the chat panel.
        let scene = app.build(app.viewport, 1.0);
        assert!(scene.texts().any(|t| t.text == "[Tests ok]"), "ok badge missing");
        assert!(scene.texts().any(|t| t.text == "[Build FAIL]"), "failed badge missing");
    }

    #[test]
    fn arrows_and_enter_route_a_focused_thread_row_to_switch_thread() {
        let mut app = App::new();
        let (cmd_tx, cmd_rx) = std::sync::mpsc::channel();
        app.command_tx = cmd_tx.clone();
        // App::new wired the view callbacks to its own channel; re-point the
        // thread-switch callback at the test channel like the swap above does
        // for paths that read `self.command_tx` directly.
        app.view.on_switch_thread = Some(Box::new(move |thread_id| {
            static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
            let id = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let _ = cmd_tx.send((id, Command::SwitchThread { thread_id }));
        }));

        app.events.lock().unwrap().push(Event::ThreadList {
            threads: vec![
                z_protocol::ThreadInfo {
                    id: "t1".into(),
                    title: "Alpha thread".into(),
                    message_count: 1,
                    updated_ms: 0,
                },
                z_protocol::ThreadInfo {
                    id: "t2".into(),
                    title: "Beta thread".into(),
                    message_count: 2,
                    updated_ms: 1,
                },
            ],
        });
        assert!(app.drain_events());

        let viewport = app.viewport;
        let row = |app: &mut App, label: &str| {
            app.build(viewport, 1.0)
                .access()
                .nodes()
                .iter()
                .find(|node| node.label == label)
                .map(|node| node.id)
                .unwrap_or_else(|| panic!("{label:?} thread row missing from the semantic tree"))
        };

        let alpha = row(&mut app, "Alpha thread");
        let beta = row(&mut app, "Beta thread");
        assert!(app.view.focus(alpha, viewport));
        assert!(app.on_key(&Key::Named(NamedKey::ArrowDown), ModifiersState::empty()));
        assert_eq!(app.view.focused(), Some(beta));

        // Enter activates the focused row; the command channel receives the
        // SwitchThread command with the row's runtime id.
        assert!(app.on_key(&Key::Named(NamedKey::Enter), ModifiersState::empty()));
        match cmd_rx.try_recv() {
            Ok((_, Command::SwitchThread { thread_id })) => assert_eq!(thread_id, "t2"),
            other => panic!("expected SwitchThread, got {other:?}"),
        }
    }
}
