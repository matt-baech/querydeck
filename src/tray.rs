//! System tray icon (StatusNotifierItem, as used by KDE and most Linux
//! desktops). Only runs while "minimize to tray" is enabled.

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

#[cfg(not(target_os = "linux"))]
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
            "litedb".into()
        }

        fn title(&self) -> String {
            "litedb".into()
        }

        fn icon_name(&self) -> String {
            "server-database".into()
        }

        fn tool_tip(&self) -> ksni::ToolTip {
            ksni::ToolTip { title: "litedb".into(), description: "SQL client".into(), ..Default::default() }
        }

        fn activate(&mut self, _x: i32, _y: i32) {
            let _ = self.events.send(TrayEvent::Toggle);
        }

        fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
            use ksni::menu::StandardItem;
            vec![
                StandardItem {
                    label: "Show litedb".into(),
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
