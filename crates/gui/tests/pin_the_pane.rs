//! Pinning a popped-out tool window to what it shows (#619).
//!
//! `pop_out_windows.rs` already proves a pane popping out and docking back;
//! this proves the one thing that is different about a *pinned* pop-out: it
//! keeps its own snapshot instead of following the shared selection every
//! other window does. A real [`gui::PaneWindows`] registry, built with
//! `gui::wire_callbacks` and `gui::wire_pop_out` - the same functions `main`
//! calls - against one shared `App` and one shared service, per CLAUDE.md
//! rule 14.

use gui::app::{App, Pane, PinId};
use gui::{MainWindow, PaneWindows, sync_pinned_window, sync_ui};
use slint::platform::WindowEvent;
use slint::{ComponentHandle, Model};
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, Instant};

mod common;
use common::ensure_service;

/// A scratch directory of this test's own, holding two files so the shared
/// selection can move from one to the other while a pinned window keeps
/// showing the first.
fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join("repos-explorer-pin").join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a scratch directory");
    std::fs::write(dir.join("a.txt"), "file a\n").expect("a.txt is written");
    std::fs::write(dir.join("b.txt"), "file b\n").expect("b.txt is written");
    dir
}

/// Whether `status` is one of the transient lines an in-flight request puts
/// up, which is how [`pump`] knows the application is still working.
fn still_working(status: &str) -> bool {
    status.starts_with("loading ") || status == "working..."
}

/// Ticks the application and syncs every open window, the way `main`'s
/// 100ms timer does - a pinned window from its own snapshot, every other
/// window from the shared selection, exactly as `main.rs` chooses between
/// them.
fn pump(windows: &Rc<RefCell<PaneWindows>>, app: &Rc<RefCell<App>>) {
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut quiet = 0u32;
    loop {
        assert!(Instant::now() < deadline, "the application never settled");
        let (busy, status) = {
            let mut app = app.borrow_mut();
            app.tick();
            let windows = windows.borrow();
            for (ui, pin) in windows.windows_with_pin() {
                match pin {
                    Some(id) if app.is_pinned(id) => sync_pinned_window(ui, &app, id),
                    _ => sync_ui(ui, &app),
                }
            }
            (app.is_busy(), windows.main().get_status_text().to_string())
        };
        if busy || still_working(&status) {
            quiet = 0;
        } else {
            quiet += 1;
            if quiet >= 8 {
                return;
            }
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

thread_local! {
    /// `init_no_event_loop` sets the testing backend for the calling
    /// thread; a second call on the same thread panics rather than being
    /// a no-op, and this file - like `pop_out_windows.rs` - opens more
    /// than one window (and so calls `windows_at` more than once) per
    /// `#[test]`.
    static TESTING_BACKEND_READY: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// A shown main window on a real `App` rooted at `root`, registered as the
/// only entry of a fresh [`PaneWindows`], wired exactly as `main` wires it.
fn windows_at(root: &Path) -> (Rc<RefCell<PaneWindows>>, Rc<RefCell<App>>) {
    ensure_service();
    TESTING_BACKEND_READY.with(|ready| {
        if !ready.get() {
            i_slint_backend_testing::init_no_event_loop();
            ready.set(true);
        }
    });
    let app = Rc::new(RefCell::new(App::new(root.to_path_buf())));
    let ui = MainWindow::new().expect("the window should build");
    gui::wire_callbacks(&ui, &app);
    sync_ui(&ui, &app.borrow());
    ui.show().expect("the window should show");
    ui.window()
        .dispatch_event(WindowEvent::WindowActiveChanged(true));
    let windows = Rc::new(RefCell::new(PaneWindows::new(ui)));
    gui::wire_pop_out(windows.borrow().main(), None, &windows, &app);
    pump(&windows, &app);
    (windows, app)
}

/// A strong handle to the window holding `pane` right now, gotten and
/// dropped out of `windows`'s borrow in one statement - so the caller can
/// invoke a callback on it without still holding that borrow, which a
/// pop-out, dock or pin callback re-enters.
fn pane_handle(windows: &Rc<RefCell<PaneWindows>>, pane: Pane) -> MainWindow {
    windows
        .borrow()
        .window_for(pane)
        .as_weak()
        .upgrade()
        .expect("the window has not been dropped yet")
}

/// The window a pin has ever opened, and its id - `None` until one has.
/// There is at most one in every test here, so the first `windows_with_pin`
/// reports is the one.
fn pinned_window(windows: &Rc<RefCell<PaneWindows>>) -> Option<(PinId, MainWindow)> {
    windows.borrow().windows_with_pin().find_map(|(ui, pin)| {
        pin.map(|id| {
            (
                id,
                ui.as_weak()
                    .upgrade()
                    .expect("the window has not been dropped yet"),
            )
        })
    })
}

/// The Contents row named `name`, for a click that does not depend on
/// today's sort order.
fn row_of(ui: &MainWindow, name: &str) -> i32 {
    let rows = ui.get_content_rows();
    i32::try_from(
        (0..rows.row_count())
            .find(|&index| rows.row_data(index).expect("a row").name == name)
            .unwrap_or_else(|| panic!("no row named {name}")),
    )
    .expect("a row index fits in i32")
}

/// Pops the File tool slot out (from whichever window currently holds it)
/// and pins it, with `name` selected in `main` first - the common start of
/// every test below.
fn pop_out_and_pin(
    windows: &Rc<RefCell<PaneWindows>>,
    app: &Rc<RefCell<App>>,
    main: &MainWindow,
    name: &str,
) -> MainWindow {
    main.invoke_content_row_clicked(row_of(main, name));
    pump(windows, app);
    pane_handle(windows, Pane::File).invoke_pop_out_requested(2);
    pump(windows, app);
    let popped = pane_handle(windows, Pane::File);
    assert!(!popped.get_pinned(), "not pinned yet");
    popped.invoke_pin_requested();
    pump(windows, app);
    assert!(popped.get_pinned(), "pinning should have taken");
    popped
}

/// #619 requirements 2, 4 and 5: pinning keeps what was showing while the
/// shared selection moves on; popping the tool out again opens a fresh,
/// still-live window rather than reusing the pinned one; and unpinning
/// returns to following the shared selection straight away.
#[test]
fn pinning_keeps_its_file_while_the_shared_selection_moves_on_and_unpinning_follows_it_again() {
    let (windows, app) = windows_at(&scratch("pin-basic"));
    let main = windows.borrow().main().as_weak().upgrade().unwrap();

    let popped = pop_out_and_pin(&windows, &app, &main, "a.txt");
    assert!(
        popped.get_file_text().contains("file a"),
        "pinned on a.txt: {}",
        popped.get_file_text()
    );
    assert!(
        popped.get_window_title().contains("a.txt"),
        "the window's title gains the pinned item's name: {}",
        popped.get_window_title()
    );

    // The shared selection moves on to b.txt, in the main window.
    main.invoke_content_row_clicked(row_of(&main, "b.txt"));
    pump(&windows, &app);

    assert!(
        popped.get_file_text().contains("file a"),
        "the pinned window still shows a.txt: {}",
        popped.get_file_text()
    );

    // Popping the File tool out again does not reuse the pinned window -
    // it opens another one, which follows the shared selection.
    let live = pane_handle(&windows, Pane::File);
    assert!(
        !std::ptr::eq(&raw const live, &raw const popped),
        "a fresh window, not the pinned one"
    );
    assert!(
        live.get_file_text().contains("file b"),
        "and it shows the shared selection: {}",
        live.get_file_text()
    );

    // Unpinning returns the pinned window to following the shared
    // selection straight away.
    popped.invoke_unpin_requested();
    pump(&windows, &app);
    assert!(!popped.get_pinned());
    assert!(
        popped.get_file_text().contains("file b"),
        "unpinned, it now shows the shared selection too: {}",
        popped.get_file_text()
    );
}

/// #619 requirement 7: docking a pinned window unpins it - and, since a
/// pinned window has no pane slot of its own left in the main window to
/// return into, closes it, the same as its own close button would.
#[test]
fn docking_a_pinned_window_closes_it() {
    let (windows, app) = windows_at(&scratch("pin-dock"));
    let main = windows.borrow().main().as_weak().upgrade().unwrap();
    let popped = pop_out_and_pin(&windows, &app, &main, "a.txt");
    assert!(pinned_window(&windows).is_some());

    popped.invoke_dock_requested(2);
    pump(&windows, &app);

    assert!(
        pinned_window(&windows).is_none(),
        "docking closed the pinned window"
    );
    assert!(!popped.window().is_visible());
    // The main window's own File pane stays exactly as it was: hidden
    // since the original pop-out, and not this window's slot to give
    // back. Popping the tool out again still works, into a fresh window.
    assert!(!main.get_show_file_pane());
    pane_handle(&windows, Pane::File).invoke_pop_out_requested(2);
    pump(&windows, &app);
    assert!(pane_handle(&windows, Pane::File).get_show_file_pane());
}

/// The platform's own close button reaches the same place as the Dock
/// button does for a pinned window (#619 requirement 7), the same join
/// `pop_out_windows.rs`'s `closing_a_popped_out_window_docks_its_pane_back`
/// proves for an ordinary one.
#[test]
fn closing_a_pinned_window_from_its_own_decoration_closes_it_too() {
    let (windows, app) = windows_at(&scratch("pin-close"));
    let main = windows.borrow().main().as_weak().upgrade().unwrap();
    let popped = pop_out_and_pin(&windows, &app, &main, "a.txt");

    popped.window().dispatch_event(WindowEvent::CloseRequested);
    pump(&windows, &app);

    assert!(pinned_window(&windows).is_none());
    assert!(!popped.window().is_visible());
}

/// #619 requirement 6: a pinned file deleted through the application - from
/// the main window's Contents pane, while the pinned window shows a
/// snapshot of it rather than the live listing - is noticed and reported,
/// and Unpin stays offered rather than the window closing on its own.
#[test]
fn deleting_the_pinned_file_through_the_application_is_noticed_and_still_offers_to_unpin() {
    let (windows, app) = windows_at(&scratch("pin-gone"));
    let main = windows.borrow().main().as_weak().upgrade().unwrap();
    let popped = pop_out_and_pin(&windows, &app, &main, "a.txt");
    let (id, _) = pinned_window(&windows).expect("the window is pinned");

    // a.txt is still the lead selection in Contents - deleting it is the
    // ordinary Delete confirmation flow, run directly on the shared `App`
    // the same way the keyboard would.
    {
        let mut app = app.borrow_mut();
        app.request_delete();
        app.confirm_delete();
    }
    pump(&windows, &app);

    assert!(app.borrow().pinned_gone(id), "the pinned path is gone");
    assert!(
        popped.get_file_text().contains("is no longer there"),
        "the pinned window says so: {}",
        popped.get_file_text()
    );
    assert!(
        popped.get_pinned(),
        "still pinned - Unpin stays offered rather than the window \
         closing on its own"
    );

    // Unpin still works from here.
    popped.invoke_unpin_requested();
    pump(&windows, &app);
    assert!(!popped.get_pinned());
}
