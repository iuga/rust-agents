use anyhow::{Context, Result};
use gpui_kit::{AssetSource, assets::Assets};
use tokio::sync::mpsc;
use tray_icon::{
    Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent,
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
fn app_image() -> Result<resvg::tiny_skia::Pixmap> {
    let tree = resvg::usvg::Tree::from_data(
        include_bytes!("app-icon-background.svg"),
        &resvg::usvg::Options::default(),
    )?;
    let mut image = resvg::tiny_skia::Pixmap::new(512, 512).context("Cannot allocate app icon")?;
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::identity(),
        &mut image.as_mut(),
    );
    let bot = bot_image(280)?;
    image.draw_pixmap(
        116,
        116,
        bot.as_ref(),
        &resvg::tiny_skia::PixmapPaint::default(),
        resvg::tiny_skia::Transform::identity(),
        None,
    );
    Ok(image)
}

#[cfg(target_os = "macos")]
pub fn set_app_icon() -> Result<()> {
    use objc2::{AllocAnyThread, MainThreadMarker};
    use objc2_app_kit::{NSApplication, NSImage};
    use objc2_foundation::NSData;

    let main_thread = MainThreadMarker::new().context("App icon must be set on the main thread")?;
    let png = app_image()?.encode_png()?;
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
        .with_menu_on_left_click(false)
        .build()?;
    let tray_id = tray.id().clone();
    let click_commands = commands.clone();
    TrayIconEvent::set_event_handler(Some(move |event: TrayIconEvent| {
        if let TrayIconEvent::Click {
            id,
            button: MouseButton::Left,
            button_state: MouseButtonState::Up,
            ..
        } = event
            && id == tray_id
        {
            let _ = click_commands.send(Command::Open);
        }
    }));
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

    #[cfg(target_os = "macos")]
    #[test]
    fn app_icon_has_white_background_black_symbol_and_transparent_corners() {
        let image = super::app_image().unwrap();
        assert_eq!((image.width(), image.height()), (512, 512));
        for (x, y) in [(0, 0), (511, 511), (50, 50), (461, 461)] {
            assert_eq!(image.pixel(x, y).unwrap().alpha(), 0);
        }
        let white = resvg::tiny_skia::PremultipliedColorU8::from_rgba(255, 255, 255, 255).unwrap();
        for (x, y) in [(256, 80), (80, 256), (431, 256), (256, 431)] {
            assert_eq!(image.pixel(x, y).unwrap(), white);
        }
        let black = resvg::tiny_skia::PremultipliedColorU8::from_rgba(0, 0, 0, 255).unwrap();
        let mut bounds = (512, 512, 0, 0);
        for y in 0..512 {
            for x in 0..512 {
                if image.pixel(x, y).unwrap() == black {
                    bounds = (
                        bounds.0.min(x),
                        bounds.1.min(y),
                        bounds.2.max(x),
                        bounds.3.max(y),
                    );
                }
            }
        }
        assert!(bounds.0 < bounds.2 && bounds.1 < bounds.3);
        assert!((bounds.0 + bounds.2).abs_diff(511) <= 2);
        assert!((bounds.1 + bounds.3).abs_diff(511) <= 2);
    }
}
