//! System tray icon. Only runs while "minimize to tray" is enabled.
//!
//! - Linux: StatusNotifierItem over D-Bus (KDE and most Linux desktops), via
//!   `ksni`.
//! - Windows and macOS: `tray-icon`, which has to live on a thread running the
//!   platform's event loop; see `imp` below for how that's arranged next to
//!   Iced's own (winit) event loop.

use iced::Subscription;

#[derive(Debug, Clone)]
pub enum TrayEvent {
    /// The tray icon is up; closing the window can now hide it.
    Ready,
    /// No tray host is available (or D-Bus failed); closing will quit.
    Unavailable(String),
    /// Left-click on the icon.
    Toggle,
    Show,
    Quit,
}

/// The tray icon for as long as this subscription is active.
pub fn subscription() -> Subscription<TrayEvent> {
    Subscription::run(imp::run)
}

#[cfg(not(any(target_os = "linux", windows, target_os = "macos")))]
mod imp {
    use super::TrayEvent;

    pub fn run() -> impl futures_util::Stream<Item = TrayEvent> {
        futures_util::stream::once(async { TrayEvent::Unavailable("not supported on this platform yet".into()) })
    }
}

#[cfg(target_os = "linux")]
mod imp {
    use futures_util::SinkExt;
    use ksni::TrayMethods;
    use tokio::sync::mpsc;

    use super::TrayEvent;

    pub fn run() -> impl futures_util::Stream<Item = TrayEvent> {
        iced::stream::channel(8, async |mut output| {
            let (tx, mut rx) = mpsc::unbounded_channel();
            let handle = match (Tray { events: tx }).spawn().await {
                Ok(handle) => handle,
                Err(e) => {
                    let _ = output.send(TrayEvent::Unavailable(e.to_string())).await;
                    return;
                }
            };
            // Removes the icon when the subscription ends (setting turned off).
            let _guard = ShutdownOnDrop(handle);
            let _ = output.send(TrayEvent::Ready).await;
            while let Some(event) = rx.recv().await {
                if output.send(event).await.is_err() {
                    break;
                }
            }
        })
    }

    struct ShutdownOnDrop(ksni::Handle<Tray>);

    impl Drop for ShutdownOnDrop {
        fn drop(&mut self) {
            // Sends the request; the tray service finishes on its own.
            drop(self.0.shutdown());
        }
    }

    struct Tray {
        events: mpsc::UnboundedSender<TrayEvent>,
    }

    impl ksni::Tray for Tray {
        fn id(&self) -> String {
            "querydeck".into()
        }

        fn title(&self) -> String {
            "querydeck".into()
        }

        fn icon_name(&self) -> String {
            "server-database".into()
        }

        fn tool_tip(&self) -> ksni::ToolTip {
            ksni::ToolTip { title: "querydeck".into(), description: "SQL client".into(), ..Default::default() }
        }

        fn activate(&mut self, _x: i32, _y: i32) {
            let _ = self.events.send(TrayEvent::Toggle);
        }

        fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
            use ksni::menu::StandardItem;
            vec![
                StandardItem {
                    label: "Show querydeck".into(),
                    icon_name: "window".into(),
                    activate: Box::new(|t: &mut Self| {
                        let _ = t.events.send(TrayEvent::Show);
                    }),
                    ..Default::default()
                }
                .into(),
                ksni::MenuItem::Separator,
                StandardItem {
                    label: "Quit".into(),
                    icon_name: "application-exit".into(),
                    activate: Box::new(|t: &mut Self| {
                        let _ = t.events.send(TrayEvent::Quit);
                    }),
                    ..Default::default()
                }
                .into(),
            ]
        }
    }
}

/// Windows and macOS, using `tray-icon`.
///
/// Iced (winit) owns the main thread's event loop, and `tray-icon` needs the
/// icon to be created, used and dropped on a thread that runs an event loop:
///
/// - Windows: any thread with a Win32 message loop works, so the icon gets a
///   dedicated thread that creates it, pumps messages (which also runs the
///   right-click menu) and drops it once it receives `WM_QUIT`.
/// - macOS: it must be the main thread, after `NSApplication` has started.
///   The icon is created and dropped from blocks queued on the main dispatch
///   queue, which winit's run loop drains once it's running, so it's never
///   created too early. The icon lives in a main-thread `thread_local`.
///
/// Clicks and menu picks arrive through `tray-icon`'s global event handlers
/// (on whichever thread runs the event loop) and are forwarded to the
/// subscription's stream.
#[cfg(any(windows, target_os = "macos"))]
mod imp {
    use std::sync::{Mutex, Once};

    use futures_util::SinkExt;
    use tokio::sync::mpsc;
    use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
    use tray_icon::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};

    use super::TrayEvent;

    const SHOW: &str = "show";
    const QUIT: &str = "quit";

    /// Where the event handlers send to. The handlers can only be installed
    /// once per process, so each run of the subscription swaps in its sender.
    static EVENTS: Mutex<Option<mpsc::UnboundedSender<TrayEvent>>> = Mutex::new(None);

    pub fn run() -> impl futures_util::Stream<Item = TrayEvent> {
        iced::stream::channel(8, async |mut output| {
            let (tx, mut rx) = mpsc::unbounded_channel();
            *EVENTS.lock().unwrap_or_else(|e| e.into_inner()) = Some(tx);
            install_handlers();

            // Removes the icon when dropped (setting turned off).
            let _guard = match platform::spawn().await {
                Ok(Ok(guard)) => guard,
                Ok(Err(e)) => {
                    let _ = output.send(TrayEvent::Unavailable(e)).await;
                    return;
                }
                Err(_) => {
                    let _ = output.send(TrayEvent::Unavailable("tray thread stopped".into())).await;
                    return;
                }
            };
            let _ = output.send(TrayEvent::Ready).await;
            while let Some(event) = rx.recv().await {
                if output.send(event).await.is_err() {
                    break;
                }
            }
        })
    }

    fn send(event: TrayEvent) {
        if let Some(tx) = EVENTS.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
            let _ = tx.send(event);
        }
    }

    fn install_handlers() {
        static ONCE: Once = Once::new();
        ONCE.call_once(|| {
            TrayIconEvent::set_event_handler(Some(|event| {
                if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } =
                    event
                {
                    send(TrayEvent::Toggle);
                }
            }));
            MenuEvent::set_event_handler(Some(|event: MenuEvent| {
                if event.id == SHOW {
                    send(TrayEvent::Show);
                } else if event.id == QUIT {
                    send(TrayEvent::Quit);
                }
            }));
        });
    }

    /// Creates the icon; must run on the event-loop thread (see above).
    fn build() -> Result<TrayIcon, String> {
        let menu = Menu::new();
        menu.append_items(&[
            &MenuItem::with_id(SHOW, "Show querydeck", true, None),
            &PredefinedMenuItem::separator(),
            &MenuItem::with_id(QUIT, "Quit", true, None),
        ])
        .map_err(|e| e.to_string())?;
        let (rgba, size) = super::icon::database(cfg!(target_os = "macos"));
        let icon = tray_icon::Icon::from_rgba(rgba, size, size).map_err(|e| e.to_string())?;
        TrayIconBuilder::new()
            .with_id("querydeck")
            .with_tooltip("querydeck")
            .with_icon(icon)
            // A black glyph that macOS recolours for light/dark menu bars.
            .with_icon_as_template(cfg!(target_os = "macos"))
            .with_menu(Box::new(menu))
            // Left-click toggles the window; the menu is on right-click.
            .with_menu_on_left_click(false)
            .build()
            .map_err(|e| e.to_string())
    }

    #[cfg(windows)]
    mod platform {
        use tokio::sync::oneshot;
        use windows_sys::Win32::System::Threading::GetCurrentThreadId;
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            DispatchMessageW, GetMessageW, MSG, PostThreadMessageW, TranslateMessage, WM_QUIT,
        };

        /// Ends the tray thread's message loop, which then drops the icon.
        pub struct Guard {
            thread_id: u32,
        }

        impl Drop for Guard {
            fn drop(&mut self) {
                unsafe { PostThreadMessageW(self.thread_id, WM_QUIT, 0, 0) };
            }
        }

        pub fn spawn() -> oneshot::Receiver<Result<Guard, String>> {
            let (ready, rx) = oneshot::channel();
            let spawned = std::thread::Builder::new().name("tray".into()).spawn(move || {
                let tray = match super::build() {
                    Ok(tray) => tray,
                    Err(e) => {
                        let _ = ready.send(Err(e));
                        return;
                    }
                };
                // The icon's window gave this thread a message queue, so the
                // guard can post to it. If nobody is waiting any more, the
                // returned guard is dropped right away and the loop ends.
                let _ = ready.send(Ok(Guard { thread_id: unsafe { GetCurrentThreadId() } }));
                let mut msg: MSG = unsafe { std::mem::zeroed() };
                while unsafe { GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) } > 0 {
                    unsafe {
                        TranslateMessage(&msg);
                        DispatchMessageW(&msg);
                    }
                }
                drop(tray);
            });
            // On failure `ready` is dropped with the closure, so the caller
            // sees the error.
            drop(spawned);
            rx
        }
    }

    #[cfg(target_os = "macos")]
    mod platform {
        use std::cell::RefCell;

        use dispatch2::DispatchQueue;
        use tokio::sync::oneshot;
        use tray_icon::TrayIcon;

        thread_local! {
            /// Only ever touched on the main thread.
            static TRAY: RefCell<Option<TrayIcon>> = const { RefCell::new(None) };
        }

        /// Drops the icon on the main thread.
        pub struct Guard;

        impl Drop for Guard {
            fn drop(&mut self) {
                DispatchQueue::main().exec_async(|| TRAY.with_borrow_mut(|tray| drop(tray.take())));
            }
        }

        pub fn spawn() -> oneshot::Receiver<Result<Guard, String>> {
            let (ready, rx) = oneshot::channel();
            // The main queue is serial, so a removal queued by an earlier
            // guard always runs before this.
            DispatchQueue::main().exec_async(move || {
                let result = super::build().map(|tray| {
                    TRAY.with_borrow_mut(|slot| *slot = Some(tray));
                    Guard
                });
                // If nobody is waiting any more, the guard drops and removes
                // the icon again.
                let _ = ready.send(result);
            });
            rx
        }
    }
}

/// The tray glyph: a database cylinder drawn at runtime, so there's no image
/// asset or decoder to ship.
/// Also the window icon, where the platform takes one from the app.
pub mod icon {
    /// RGBA pixels and edge length at the tray's size. `template` gives a black
    /// glyph for macOS's template images; otherwise it's a muted blue.
    #[cfg(any(windows, target_os = "macos", test))]
    pub fn database(template: bool) -> (Vec<u8>, u32) {
        (pixels(32, template), 32)
    }

    /// The glyph at `size`×`size`, for the window icon.
    pub fn window(size: u32) -> Vec<u8> {
        pixels(size, false)
    }

    fn pixels(size: u32, template: bool) -> Vec<u8> {
        let (r, g, b) = if template { (0, 0, 0) } else { (0x4a, 0x7f, 0xb5) };
        // The shape is laid out on a 32-unit grid.
        let scale = 32.0 / size as f32;
        let mut rgba = Vec::with_capacity((size * size * 4) as usize);
        for y in 0..size {
            for x in 0..size {
                // 4x4 supersampling for smooth edges.
                let mut hits = 0;
                for sy in 0..4 {
                    for sx in 0..4 {
                        let px = x as f32 + (sx as f32 + 0.5) / 4.0;
                        let py = y as f32 + (sy as f32 + 0.5) / 4.0;
                        hits += covered(px * scale, py * scale) as u32;
                    }
                }
                rgba.extend_from_slice(&[r, g, b, (hits * 255 / 16) as u8]);
            }
        }
        rgba
    }

    fn covered(x: f32, y: f32) -> bool {
        const CX: f32 = 16.0;
        const RX: f32 = 12.0;
        const RY: f32 = 4.5;
        const TOP: f32 = 7.0;
        const BOTTOM: f32 = 25.0;

        let nx = (x - CX) / RX;
        if nx.abs() > 1.0 {
            return false;
        }
        let in_ellipse = |cy: f32| nx * nx + ((y - cy) / RY).powi(2) <= 1.0;
        if !(in_ellipse(TOP) || in_ellipse(BOTTOM) || (TOP..=BOTTOM).contains(&y)) {
            return false;
        }
        // Gaps along the front rim of the top face and two bands below it.
        let arc = RY * (1.0 - nx * nx).sqrt();
        let band = (BOTTOM - TOP) / 3.0;
        ![TOP, TOP + band, TOP + 2.0 * band].iter().any(|cy| (y - (cy + arc)).abs() < 0.8)
    }

    #[cfg(test)]
    mod tests {
        #[test]
        fn draws_a_glyph() {
            let (rgba, size) = super::database(false);
            assert_eq!(rgba.len(), (size * size * 4) as usize);
            let alpha = |x: u32, y: u32| rgba[((y * size + x) * 4 + 3) as usize];
            assert_eq!(alpha(0, 0), 0);
            let art: String = (0..size)
                .map(|y| (0..size).map(|x| if alpha(x, y) > 127 { '#' } else { '.' }).collect::<String>() + "\n")
                .collect();
            println!("{art}");
            assert_eq!(alpha(16, 20), 255);
        }
    }
}
