use anyhow::{Context, Result};
use gpui_kit::{AssetSource, assets::Assets};
use tokio::sync::mpsc;
use tray_icon::{
    Icon, TrayIcon, TrayIconBuilder,
    menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem},
};

pub enum Command {
    Open,
    Quit,
}

pub fn bot_icon() -> Result<Icon> {
    let image = bot_image(36)?;
    // The SVG is black with transparency, so premultiplied and straight RGBA agree.
    Ok(Icon::from_rgba(image.take(), 36, 36)?)
}

fn bot_image(size: u32) -> Result<resvg::tiny_skia::Pixmap> {
    let svg = Assets.load("icons/bot.svg")?.context("Missing Bot icon")?;
    let tree = resvg::usvg::Tree::from_data(&svg, &resvg::usvg::Options::default())?;
    let mut image =
        resvg::tiny_skia::Pixmap::new(size, size).context("Cannot allocate Bot icon")?;
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::from_scale(size as f32 / 24., size as f32 / 24.),
        &mut image.as_mut(),
    );
    Ok(image)
}

#[cfg(target_os = "macos")]
pub fn set_app_icon() -> Result<()> {
    use objc2::{AllocAnyThread, MainThreadMarker};
    use objc2_app_kit::{NSApplication, NSImage};
    use objc2_foundation::NSData;

    let main_thread = MainThreadMarker::new().context("App icon must be set on the main thread")?;
    let png = bot_image(512)?.encode_png()?;
    let image = NSImage::initWithData(NSImage::alloc(), &NSData::with_bytes(&png))
        .context("Cannot create macOS app icon")?;
    // AppKit requires an image owned for the duration of this main-thread call.
    unsafe {
        NSApplication::sharedApplication(main_thread).setApplicationIconImage(Some(&image));
    }
    Ok(())
}

/// Call on the GPUI main thread after its native event loop starts.
pub fn create(commands: mpsc::UnboundedSender<Command>) -> Result<TrayIcon> {
    let menu = Menu::new();
    let open = MenuItem::new("Open Chat", true, None);
    let quit = MenuItem::new("Quit", true, None);
    menu.append_items(&[&open, &PredefinedMenuItem::separator(), &quit])?;
    let tray = TrayIconBuilder::new()
        .with_tooltip("Alpha")
        .with_icon_templated(bot_icon()?)
        .with_menu(Box::new(menu))
        .build()?;
    let open_id = open.id().clone();
    let quit_id = quit.id().clone();
    MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
        let command = if event.id == open_id {
            Command::Open
        } else if event.id == quit_id {
            Command::Quit
        } else {
            return;
        };
        let _ = commands.send(command);
    }));
    Ok(tray)
}

#[cfg(test)]
mod tests {
    #[test]
    fn bundled_bot_can_be_rendered_for_the_tray() {
        assert!(super::bot_icon().is_ok());
    }
}
