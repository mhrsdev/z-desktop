//! Host window and the frame loop.
//!
//! The scene-producing callback lives outside this module: ZeroGPUI owns the
//! window and the frame pipeline, and asks the application what to draw. That
//! direction of control is what keeps the runtime free of any knowledge about
//! chat, agents or projects.

use crate::a11y_platform::{AccessRequest, PlatformAdapter};
use crate::geometry::{Point, Rect};
use crate::renderer::{BackendInfo, FrameStats, Renderer};
use crate::scene::Scene;
use crate::text::TextSystem;
use crate::timing::{FrameBudget, FrameHistory, FrameTimer, FrameTiming, Stage, TimingSummary};
use std::sync::Arc;
use winit::application::ApplicationHandler;
use winit::event::{ElementState, KeyEvent, Modifiers, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
use winit::keyboard::ModifiersState;
use winit::keyboard::{Key, NamedKey};
use winit::window::{Window, WindowId};

/// What the host asks the application for, each frame.
pub trait SceneSource {
    /// Build the scene for a viewport of this size.
    fn build(&mut self, viewport: Rect, scale: f32) -> Scene;

    /// Colour behind everything. Read from tokens, never hardcoded here.
    fn clear_color(&self) -> z_tokens::Rgba;

    /// Handle a key press. Returns true when the frame should be rebuilt.
    ///
    /// `modifiers` carries Shift/Ctrl/Alt state, which Tab needs in order to
    /// walk focus backwards.
    fn on_key(&mut self, _key: &Key, _modifiers: ModifiersState) -> bool {
        false
    }

    /// Handle a wheel or trackpad scroll, in logical pixels. Positive is down.
    /// Returns true when the frame should be rebuilt.
    fn on_scroll(&mut self, _delta: f32) -> bool {
        false
    }

    /// Handle a primary-pointer click in logical window pixels. The runtime
    /// only forwards the position; hit testing and product commands stay in
    /// the application layer beside the semantic tree that defines them.
    fn on_click(&mut self, _position: Point) -> bool {
        false
    }

    /// Handle a request from assistive technology. Returns true when the
    /// resulting state change requires a rebuilt frame.
    fn on_access_request(&mut self, _request: AccessRequest) -> bool {
        false
    }

    /// Called once the backend is known, so it can be surfaced in diagnostics.
    fn on_backend_ready(&mut self, _info: &BackendInfo) {}

    /// Called before the loop starts, handing over a proxy that can wake the
    /// loop from another thread. Background work (an agent runtime streaming)
    /// uses this to request redraws without polling.
    fn on_ready(&mut self, _proxy: EventLoopProxy<HostEvent>) {}

    /// A wake arrived from another thread. Returns true when the frame should
    /// be rebuilt.
    fn on_wake(&mut self) -> bool {
        false
    }

    /// Called after each frame that actually drew.
    fn on_frame(&mut self, _stats: FrameStats, _timing: FrameTiming) {}

    /// Called when a frame overran its budget, with the stage that cost most.
    ///
    /// Separate from `on_frame` so an overrun is impossible to miss by
    /// accident: a host that ignores it has chosen to, rather than never
    /// having been told.
    fn on_budget_overrun(&mut self, timing: FrameTiming, budget: &FrameBudget) {
        let (stage, spent) = timing.slowest();
        log::warn!(
            "frame overran: {:.2}ms of {:.2}ms, slowest stage {} at {:.2}ms",
            timing.total.as_secs_f64() * 1000.0,
            budget.frame.as_secs_f64() * 1000.0,
            stage.name(),
            spent.as_secs_f64() * 1000.0
        );
    }

    /// Called when the window closes, with the session's frame statistics.
    fn on_session_end(&mut self, _summary: TimingSummary) {}
}

pub struct WindowConfig {
    pub title: String,
    pub width: f64,
    pub height: f64,
    /// Present a single frame and exit. Used by the screenshot and smoke paths
    /// so CI can verify the shell renders without a human watching.
    pub single_frame: bool,
}

impl Default for WindowConfig {
    fn default() -> Self {
        // Fits a 1080p panel at 125% scale without the compositor clamping the
        // request, which would otherwise silently change the size we asked for.
        Self { title: "Zero".into(), width: 1440.0, height: 810.0, single_frame: false }
    }
}

/// Logical pixels per wheel notch. Three lines of body text, which is the
/// conventional feel on every desktop platform.
const LINE_SCROLL_PIXELS: f32 = 3.0 * 22.0;

struct Host<S: SceneSource> {
    config: WindowConfig,
    source: S,
    state: Option<GraphicsState>,
    previous: Scene,
    frames: u64,
    budget: FrameBudget,
    history: FrameHistory,
    modifiers: Modifiers,
    cursor_position: Option<Point>,
    event_loop_proxy: EventLoopProxy<HostEvent>,
}

#[derive(Debug, Clone, Copy)]
pub enum HostEvent {
    Accessibility,
    /// A background thread has new state for the application.
    Wake,
}

struct GraphicsState {
    window: Arc<Window>,
    accessibility: PlatformAdapter,
    surface: wgpu::Surface<'static>,
    surface_config: wgpu::SurfaceConfiguration,
    renderer: Renderer,
    text: TextSystem,
}

impl<S: SceneSource> ApplicationHandler<HostEvent> for Host<S> {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.state.is_some() {
            return;
        }

        let attributes = Window::default_attributes()
            .with_title(self.config.title.clone())
            .with_visible(false)
            .with_inner_size(winit::dpi::LogicalSize::new(self.config.width, self.config.height))
            .with_min_inner_size(winit::dpi::LogicalSize::new(
                z_tokens::metrics::Shell::WINDOW_MIN_WIDTH as f64,
                z_tokens::metrics::Shell::WINDOW_MIN_HEIGHT as f64,
            ));

        let window = Arc::new(
            event_loop.create_window(attributes).expect("the host could not create a window"),
        );

        let proxy = self.event_loop_proxy.clone();
        let accessibility = PlatformAdapter::new(event_loop, &window, move || {
            let _ = proxy.send_event(HostEvent::Accessibility);
        });

        match pollster::block_on(GraphicsState::new(window, accessibility)) {
            Ok(state) => {
                // Pace to the display, not to an assumed 60 Hz. A 144 Hz panel
                // gets a 6.9ms budget; a 60 Hz one gets 16.6ms.
                if let Some(hz) = state
                    .window
                    .current_monitor()
                    .and_then(|monitor| monitor.refresh_rate_millihertz())
                {
                    self.budget = FrameBudget::for_refresh_rate(
                        hz as f32 / 1000.0,
                        self.budget.input_to_present,
                    );
                    log::info!("frame budget: {:.1} Hz", hz as f32 / 1000.0);
                }
                self.source.on_backend_ready(state.renderer.backend_info());
                state.window.set_visible(true);
                state.window.request_redraw();
                self.state = Some(state);
            }
            Err(error) => {
                log::error!("ZeroRender could not start: {error}");
                event_loop.exit();
            }
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        let Some(state) = self.state.as_mut() else { return };

        // AccessKit must see the original event before any application state
        // changes in response to it.
        state.accessibility.process_event(&state.window, &event);

        match event {
            WindowEvent::CloseRequested => event_loop.exit(),

            WindowEvent::Resized(size) => {
                state.resize(size.width, size.height);
                // A resize invalidates the whole surface, so drop the comparison
                // scene rather than diffing against a differently sized frame.
                self.previous = Scene::new();
                state.window.request_redraw();
            }

            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                state.text.set_scale(scale_factor as f32);
                self.previous = Scene::new();
                state.window.request_redraw();
            }

            WindowEvent::ModifiersChanged(modifiers) => {
                self.modifiers = modifiers;
            }

            WindowEvent::MouseWheel { delta, .. } => {
                // Wheel notches and trackpad pixels arrive in different units;
                // normalise to logical pixels so the app sees one thing.
                let pixels = match delta {
                    MouseScrollDelta::LineDelta(_, lines) => -lines * LINE_SCROLL_PIXELS,
                    MouseScrollDelta::PixelDelta(position) => -position.y as f32,
                };
                if self.source.on_scroll(pixels) {
                    state.window.request_redraw();
                }
            }

            WindowEvent::CursorMoved { position, .. } => {
                // Winit reports a physical-pixel cursor position while the
                // scene and semantic bounds use logical pixels.
                let scale = state.window.scale_factor() as f32;
                self.cursor_position =
                    Some(Point::new(position.x as f32 / scale, position.y as f32 / scale));
            }

            WindowEvent::CursorLeft { .. } => {
                self.cursor_position = None;
            }

            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                button: MouseButton::Left,
                ..
            } => {
                if let Some(position) = self.cursor_position {
                    if self.source.on_click(position) {
                        state.window.request_redraw();
                    }
                }
            }

            WindowEvent::KeyboardInput {
                event: KeyEvent { logical_key, state: ElementState::Pressed, .. },
                ..
            } => {
                // The application gets Escape first: it may mean "cancel the
                // running turn" or "dismiss this approval" rather than quit.
                // Only an unhandled Escape closes the window.
                if logical_key == Key::Named(NamedKey::Escape) {
                    if !self.source.on_key(&logical_key, self.modifiers.state()) {
                        event_loop.exit();
                    } else {
                        state.window.request_redraw();
                    }
                    return;
                }
                if self.source.on_key(&logical_key, self.modifiers.state()) {
                    state.window.request_redraw();
                }
            }

            WindowEvent::RedrawRequested => {
                let mut timer = FrameTimer::start();

                let scale = state.window.scale_factor() as f32;
                let viewport = Rect::new(
                    0.0,
                    0.0,
                    state.surface_config.width as f32 / scale,
                    state.surface_config.height as f32 / scale,
                );

                timer.begin(Stage::Update);
                let scene = self.source.build(viewport, scale);
                state.accessibility.update(scene.access(), scale);

                timer.begin(Stage::SceneDiff);
                let damage = scene.damage_against(&self.previous);
                timer.end();

                if damage.is_some() || self.frames == 0 {
                    use wgpu::CurrentSurfaceTexture as Acquired;
                    match state.surface.get_current_texture() {
                        Acquired::Success(frame) | Acquired::Suboptimal(frame) => {
                            let view =
                                frame.texture.create_view(&wgpu::TextureViewDescriptor::default());

                            timer.begin(Stage::Render);
                            let stats = state.renderer.render(
                                &view,
                                &scene,
                                &mut state.text,
                                viewport,
                                self.source.clear_color(),
                                // Force the very first frame, which has nothing
                                // to diff against.
                                damage.or(Some(viewport)),
                            );

                            // wgpu 30 presents through the queue, after the
                            // frame's commands have been submitted.
                            timer.begin(Stage::Present);
                            state.renderer.queue().present(frame);

                            let timing = timer.finish();
                            self.history.record(timing, &self.budget);
                            if self.budget.frame_overran(&timing) {
                                self.source.on_budget_overrun(timing, &self.budget);
                            }
                            self.source.on_frame(stats, timing);

                            self.previous = scene;
                            self.frames += 1;
                        }
                        // Losing the device or outliving the surface config is a
                        // normal event on a desktop — a driver reset, a monitor
                        // change — not a crash. Reconfigure and try again.
                        Acquired::Lost | Acquired::Outdated => {
                            let (w, h) = (state.surface_config.width, state.surface_config.height);
                            state.resize(w, h);
                            state.window.request_redraw();
                        }
                        // The compositor is not showing us; skip the frame
                        // rather than burning a GPU submit on nothing.
                        Acquired::Occluded => {}
                        Acquired::Timeout => {
                            log::warn!("surface acquire timed out; skipping this frame");
                        }
                        Acquired::Validation => {
                            log::error!(
                                "surface rejected the configuration; shutting down cleanly"
                            );
                            event_loop.exit();
                        }
                    }
                }

                if self.config.single_frame && self.frames > 0 {
                    event_loop.exit();
                }
            }

            _ => {}
        }
    }

    fn user_event(&mut self, _event_loop: &ActiveEventLoop, event: HostEvent) {
        match event {
            HostEvent::Accessibility => {
                let Some(state) = self.state.as_ref() else { return };
                let requests = state.accessibility.take_requests();
                let mut changed = false;
                for request in requests {
                    changed |= self.source.on_access_request(request);
                }
                if changed {
                    state.window.request_redraw();
                }
            }
            HostEvent::Wake => {
                if self.source.on_wake() {
                    if let Some(state) = self.state.as_ref() {
                        state.window.request_redraw();
                    }
                }
            }
        }
    }
}

impl GraphicsState {
    async fn new(window: Arc<Window>, accessibility: PlatformAdapter) -> Result<Self, String> {
        let size = window.inner_size();
        let scale = window.scale_factor() as f32;

        // All three target backends; which one is used is decided by what the
        // machine actually reports, never assumed at compile time.
        let mut descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
        descriptor.backends = wgpu::Backends::PRIMARY;
        let instance = wgpu::Instance::new(descriptor);

        let surface = instance
            .create_surface(window.clone())
            .map_err(|e| format!("no drawable surface: {e}"))?;

        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                // The UI has no business claiming the discrete GPU: that is
                // where local inference lives.
                power_preference: wgpu::PowerPreference::LowPower,
                compatible_surface: Some(&surface),
                force_fallback_adapter: false,
                ..Default::default()
            })
            .await
            .map_err(|e| format!("no compatible GPU adapter: {e}"))?;

        let info = adapter.get_info();
        let backend_info = BackendInfo {
            backend: format!("{:?}", info.backend),
            adapter: info.name.clone(),
            device_type: format!("{:?}", info.device_type),
        };

        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("zero.device"),
                required_features: wgpu::Features::empty(),
                // Downlevel defaults so the same binary runs on the low tier.
                required_limits: wgpu::Limits::downlevel_defaults()
                    .using_resolution(adapter.limits()),
                memory_hints: wgpu::MemoryHints::Performance,
                trace: wgpu::Trace::Off,
                ..Default::default()
            })
            .await
            .map_err(|e| format!("could not acquire a device: {e}"))?;

        // Start from the surface's own reported defaults rather than inventing a
        // configuration, then override only what the design actually requires.
        let mut surface_config = surface
            .get_default_config(&adapter, size.width.max(1), size.height.max(1))
            .ok_or_else(|| "the surface is not usable with this adapter".to_string())?;

        // sRGB where offered: token colours are authored in sRGB, and letting
        // the surface do the encode keeps blending linear.
        let capabilities = surface.get_capabilities(&adapter);
        if let Some(srgb) = capabilities.formats.iter().copied().find(|f| f.is_srgb()) {
            surface_config.format = srgb;
        }
        // Fifo is supported everywhere and paces to the display's own refresh
        // rate — no artificial 60 FPS cap.
        surface_config.present_mode = wgpu::PresentMode::Fifo;
        surface_config.usage = wgpu::TextureUsages::RENDER_ATTACHMENT;

        surface.configure(&device, &surface_config);
        let format = surface_config.format;

        let text = TextSystem::new(scale);
        let atlas_size = text.atlas().width();
        let renderer = Renderer::new(device, queue, format, backend_info, atlas_size);

        Ok(Self { window, accessibility, surface, surface_config, renderer, text })
    }

    fn resize(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
        self.surface_config.width = width;
        self.surface_config.height = height;
        self.surface.configure(self.renderer.device(), &self.surface_config);
    }
}

/// Open a window and run the frame loop until the user closes it.
pub fn run<S: SceneSource>(config: WindowConfig, source: S) -> Result<(), String> {
    let event_loop = EventLoop::<HostEvent>::with_user_event()
        .build()
        .map_err(|e| format!("no event loop: {e}"))?;
    // Wait rather than poll: an idle workspace should cost nothing.
    event_loop.set_control_flow(ControlFlow::Wait);

    // Budget defaults to the mid tier until the display's real refresh rate is
    // known; `GraphicsState::new` refines it once the monitor reports one.
    let event_loop_proxy = event_loop.create_proxy();
    let mut host = Host {
        config,
        source,
        state: None,
        previous: Scene::new(),
        frames: 0,
        budget: FrameBudget::tier_m(),
        modifiers: Modifiers::default(),
        cursor_position: None,
        event_loop_proxy,
        // A few seconds of frames at high refresh: enough for a meaningful p99,
        // bounded so a session running for hours cannot grow without limit.
        history: FrameHistory::new(1024),
    };

    host.source.on_ready(event_loop.create_proxy());

    let result = event_loop.run_app(&mut host).map_err(|e| format!("event loop stopped: {e}"));

    if !host.history.is_empty() {
        let summary = host.history.summary();
        log::info!("session frames: {summary}");
        host.source.on_session_end(summary);
    }

    result
}
