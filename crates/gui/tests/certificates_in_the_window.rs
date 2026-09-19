//! The Certificates tool (#622), driven end to end through a real window on
//! a real [`App`] - `service`'s own tests already prove `find_certificates`
//! walks a Repos Directory correctly, and `app`'s own tests prove the
//! table's flattening, sort and counts against hand-built responses
//! (`Request::FindCertificates` reads the machine's configured Repos
//! Directory rather than taking one as a parameter, the same as
//! `Request::FindNames`, so a real end-to-end walk through a shared,
//! process-wide config file is not what a window test can afford to
//! depend on without risking flakiness against whatever else is running).
//! What neither of those halves can see is the seam between them: View >
//! Certificates clicked in a real menu, the tool slot drawing a real table
//! from what `App` reports, a row's click reaching Contents, and the pane
//! host popping the tool out and docking it back with no pop-out or dock
//! code of the tool's own (grep `crates/gui/ui/panes/certificates_view.slint`
//! for `pop-out`/`dock`/`pin`: none). That seam is what this file measures,
//! per CLAUDE.md rule 14.

use gui::app::App;
use gui::{MainWindow, PaneWindows, sync_ui};
use i_slint_backend_testing::ElementHandle;
use slint::platform::{PointerEventButton, WindowEvent};
use slint::{ComponentHandle, Model as _};
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::{Mutex, MutexGuard};

mod common;
use common::ensure_service;

/// A scratch directory of this test's own, holding a `.pem` file nested a
/// folder down (so selecting its row has somewhere real to navigate to)
/// One window and one service at a time: Slint's testing backend is a
/// process-wide platform, and these tests share a service. Without this
/// they pass alone and fail together - which is how the fifth test added
/// here first showed up (the review of #663).
static SERIAL: Mutex<()> = Mutex::new(());

/// Takes the shared lock, tolerating a previous test having panicked while
/// holding it - a poisoned lock would otherwise turn one failure into many.
fn serially() -> MutexGuard<'static, ()> {
    SERIAL
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// and a plain `.rs` file the Certificates tool has nothing to say about.
fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir()
        .join("rse-certificates-in-the-window")
        .join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("sub")).expect("a scratch directory");
    std::fs::write(dir.join("sub").join("leaf.pem"), "placeholder").expect("a scratch file");
    std::fs::write(dir.join("main.rs"), "fn main() {}").expect("a scratch file");
    dir
}

thread_local! {
    static TESTING_BACKEND_READY: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

fn init_backend() {
    TESTING_BACKEND_READY.with(|ready| {
        if !ready.get() {
            i_slint_backend_testing::init_no_event_loop();
            ready.set(true);
        }
    });
}

/// A shown window on a real `App` rooted at `root`, wired the way `main`
/// wires it, with the root's own listing already settled.
fn window_at(root: &Path) -> (MainWindow, Rc<RefCell<App>>) {
    ensure_service();
    init_backend();
    let app = Rc::new(RefCell::new(App::new(root.to_path_buf())));
    let ui = MainWindow::new().expect("the window should build");
    sync_ui(&ui, &app.borrow());
    gui::wire_callbacks(&ui, &app);
    ui.show().expect("the window should show");
    ui.window()
        .dispatch_event(WindowEvent::WindowActiveChanged(true));
    settle(&ui, &app);
    (ui, app)
}

/// Whether `status` is one of the transient lines an in-flight request puts
/// up, which is how [`settle`] knows the application is still working.
fn still_working(status: &str) -> bool {
    status.starts_with("loading ") || status == "working..."
}

/// Ticks the application the way the window's 100ms timer does, until
/// every in-flight request has landed and stayed landed. Only used for
/// navigation, which needs the real directory listing it asks for - never
/// after seeding a Certificates answer with
/// [`App::apply_certificates_result_for_test`], since a tick this file
/// never otherwise needs might also drain the real (and, on a machine with
/// no Repos Directory configured, merely an error - `Request::FindCertificates`
/// has no parameter of its own to scope it to this test's fixture, the
/// same as `Request::FindNames`) answer to the request
/// `App::open_certificates_tool` already sent for real.
fn settle(ui: &MainWindow, app: &Rc<RefCell<App>>) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    let mut quiet = 0u32;
    while std::time::Instant::now() < deadline {
        {
            let mut app = app.borrow_mut();
            app.tick();
            sync_ui(ui, &app);
        }
        if still_working(&ui.get_status_text()) {
            quiet = 0;
        } else {
            quiet += 1;
            if quiet >= 20 {
                return;
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    panic!(
        "the application never settled; status: {}",
        ui.get_status_text()
    );
}

/// Every menu item currently on screen, in whichever menu is open.
fn open_items(ui: &MainWindow) -> Vec<ElementHandle> {
    ElementHandle::find_by_element_type_name(ui, "ContextMenuItem").collect()
}

/// The open menu's item labelled `label`.
fn item(ui: &MainWindow, label: &str) -> ElementHandle {
    let mut found: Vec<ElementHandle> = open_items(ui)
        .into_iter()
        .filter(|item| item.accessible_label().is_some_and(|found| found == label))
        .collect();
    assert_eq!(
        found.len(),
        1,
        "exactly one open menu item should be labelled {label:?}"
    );
    found.remove(0)
}

/// Opens the menu-bar menu titled `title`, the topmost element carrying
/// that label.
fn open_menu(ui: &MainWindow, title: &str) {
    let mut titles: Vec<ElementHandle> =
        ElementHandle::find_by_accessible_label(ui, title).collect();
    titles.sort_by(|a, b| {
        a.absolute_position()
            .y
            .partial_cmp(&b.absolute_position().y)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    titles
        .first()
        .unwrap_or_else(|| panic!("the menu bar has no title {title:?}"))
        .mock_single_click(PointerEventButton::Left);
    assert!(
        !open_items(ui).is_empty(),
        "clicking the {title:?} title should have opened a menu"
    );
}

/// Opens View > Certificates through the real menu - what `open_menu` and
/// `item` above exist for.
fn open_certificates_from_the_menu(ui: &MainWindow) {
    open_menu(ui, "View");
    item(ui, "Certificates").mock_single_click(PointerEventButton::Left);
}

/// A `Response::Certificates` with three findings: one expired, one
/// expiring within 30 days, one comfortably valid - and a private key
/// alongside the expiring one.
fn sample_certificates(now: i64) -> protocol::Response {
    protocol::Response::Certificates {
        certificates: vec![
            protocol::CertificateFinding {
                path: "sub/leaf.pem".to_owned(),
                repository: None,
                kind: protocol::CertificateFindingKind::Blocks(vec![
                    protocol::CertificateBlock::Certificate(protocol::CertificateSummary {
                        subject: "CN=expiring.example.com".to_owned(),
                        issuer: "CN=expiring.example.com".to_owned(),
                        serial: "01".to_owned(),
                        not_before: now - 1_000,
                        not_after: now + 5 * 86_400,
                        self_signed: true,
                    }),
                    protocol::CertificateBlock::PrivateKey,
                ]),
            },
            protocol::CertificateFinding {
                path: "expired.pem".to_owned(),
                repository: None,
                kind: protocol::CertificateFindingKind::Blocks(vec![
                    protocol::CertificateBlock::Certificate(protocol::CertificateSummary {
                        subject: "CN=expired.example.com".to_owned(),
                        issuer: "CN=expired.example.com".to_owned(),
                        serial: "02".to_owned(),
                        not_before: now - 2_000,
                        not_after: now - 1_000,
                        self_signed: true,
                    }),
                ]),
            },
            protocol::CertificateFinding {
                path: "valid.pem".to_owned(),
                repository: Some("valid-repo".to_owned()),
                kind: protocol::CertificateFindingKind::Blocks(vec![
                    protocol::CertificateBlock::Certificate(protocol::CertificateSummary {
                        subject: "CN=valid.example.com".to_owned(),
                        issuer: "CN=valid.example.com".to_owned(),
                        serial: "03".to_owned(),
                        not_before: now - 1_000,
                        not_after: now + 400 * 86_400,
                        self_signed: true,
                    }),
                ]),
            },
        ],
        complete: true,
    }
}

fn now_epoch_seconds() -> i64 {
    i64::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("the clock is after 1970")
            .as_secs(),
    )
    .expect("seconds since epoch fits in an i64 for a very long time yet")
}

#[test]
fn view_certificates_shows_rows_the_default_order_and_the_counts_line() {
    let _serial = serially();
    let root = scratch("rows-order-counts");
    let (ui, app) = window_at(&root);

    open_certificates_from_the_menu(&ui);
    // The real click already asked the real service (`app.tick()` is never
    // called in this test, so that answer - which depends on whatever
    // Repos Directory the machine running this test has configured, if
    // any - is never consumed); this plants the answer this test actually
    // means to check instead, the way `tool_slot_in_the_window.rs` seeds
    // its own tool's content.
    app.borrow_mut()
        .apply_certificates_result_for_test(sample_certificates(now_epoch_seconds()));
    sync_ui(&ui, &app.borrow());

    assert!(ui.get_tool_showing_certificates());
    let rows = ui.get_certificate_rows();
    assert_eq!(rows.row_count(), 3);
    let statuses: Vec<String> = (0..rows.row_count())
        .map(|i| rows.row_data(i).unwrap().status.to_string())
        .collect();
    assert_eq!(
        statuses,
        vec![
            "Expired".to_owned(),
            "Expires in 5 days".to_owned(),
            "Valid".to_owned()
        ],
        "expired first, then soonest to expire (#622 requirement 2)"
    );
    assert!(rows.row_data(0).unwrap().status_warning);
    assert!(rows.row_data(1).unwrap().status_warning);
    assert!(!rows.row_data(2).unwrap().status_warning);
    assert_eq!(rows.row_data(2).unwrap().repository, "valid-repo");
    assert_eq!(
        ui.get_certificates_summary_line().as_str(),
        "1 expired, 1 expiring within 30 days, 1 valid"
    );
    assert_eq!(
        ui.get_certificates_private_key_line().as_str(),
        "1 private key committed"
    );
}

#[test]
fn selecting_a_row_selects_that_file_in_contents_and_the_tool_stays_active() {
    let _serial = serially();
    let root = scratch("select-a-row");
    let (ui, app) = window_at(&root);

    open_certificates_from_the_menu(&ui);
    app.borrow_mut()
        .apply_certificates_result_for_test(sample_certificates(now_epoch_seconds()));
    sync_ui(&ui, &app.borrow());

    // Row 0 is `sub/leaf.pem` (default sort: expired first is `expired.pem`
    // at row 0 in the earlier test; here only this row's own identity
    // matters, read back from the table rather than assumed).
    let rows = ui.get_certificate_rows();
    let leaf_index = (0..rows.row_count())
        .find(|&i| rows.row_data(i).unwrap().file == "sub/leaf.pem")
        .expect("sub/leaf.pem should be a row");
    ui.invoke_certificate_row_activated(i32::try_from(leaf_index).unwrap());
    settle(&ui, &app);

    assert_eq!(
        PathBuf::from(app.borrow().current_path()),
        root.join("sub"),
        "Folders should have navigated to the file's own folder"
    );
    assert!(
        ui.get_content_rows()
            .iter()
            .any(|row| row.name == "leaf.pem" && row.selected),
        "Contents should select leaf.pem"
    );
    assert!(
        ui.get_tool_showing_certificates(),
        "the Certificates tool stays active in the slot (#622 requirement 4)"
    );
}

#[test]
fn the_picker_offers_certificates_for_a_pem_file_and_not_for_an_rs_file() {
    let _serial = serially();
    let root = scratch("picker");
    let (ui, app) = window_at(&root);

    let pem_index = app
        .borrow()
        .content_rows()
        .iter()
        .position(|row| row.name == "sub/")
        .expect("the sub folder is listed");
    app.borrow_mut().select_content(pem_index);
    app.borrow_mut().open_content(pem_index);
    settle(&ui, &app);
    let leaf_index = app
        .borrow()
        .content_rows()
        .iter()
        .position(|row| row.name == "leaf.pem")
        .expect("leaf.pem is listed inside sub/");
    app.borrow_mut().select_content(leaf_index);
    sync_ui(&ui, &app.borrow());

    let titles: Vec<String> = (0..ui.get_tool_titles().row_count())
        .map(|i| ui.get_tool_titles().row_data(i).unwrap().to_string())
        .collect();
    assert_eq!(
        titles,
        vec!["File".to_owned(), "Certificates".to_owned()],
        "both tools apply to a .pem file"
    );

    // Back to the root, and select the .rs file: only the editor applies.
    app.borrow_mut().navigate_to_parent();
    settle(&ui, &app);
    let rs_index = app
        .borrow()
        .content_rows()
        .iter()
        .position(|row| row.name == "main.rs")
        .expect("main.rs is listed");
    app.borrow_mut().select_content(rs_index);
    sync_ui(&ui, &app.borrow());

    assert_eq!(
        ui.get_tool_titles().row_count(),
        0,
        "only the editor applies to a .rs file, so the picker has nothing to offer"
    );
}

/// Choosing Certificates from the picker - #622 requirement 1's other
/// entry point - has to start the search, not show an empty table for
/// ever. It did: the picker set the active tool and nothing else, so the
/// pane reported "0 expired, 0 expiring within 30 days, 0 valid" whatever
/// was on disk, never asked the service anything, and F5 did not help
/// either (the review of #663). Every other test here goes in through the
/// View menu, which is why nothing caught it.
#[test]
fn choosing_certificates_from_the_picker_starts_the_search() {
    let _serial = serially();
    let root = scratch("picker-loads");
    let (ui, app) = window_at(&root);

    let sub = app
        .borrow()
        .content_rows()
        .iter()
        .position(|row| row.name == "sub/")
        .expect("the sub folder is listed");
    app.borrow_mut().select_content(sub);
    app.borrow_mut().open_content(sub);
    settle(&ui, &app);
    let leaf = app
        .borrow()
        .content_rows()
        .iter()
        .position(|row| row.name == "leaf.pem")
        .expect("leaf.pem is listed inside sub/");
    app.borrow_mut().select_content(leaf);
    sync_ui(&ui, &app.borrow());

    // The picker's own click: "Certificates" is its second entry for a
    // certificate file, after the editor.
    ui.invoke_tool_selected(1);
    sync_ui(&ui, &app.borrow());

    assert!(
        app.borrow().active_tool_is_certificates(),
        "the picker chose the Certificates tool"
    );
    assert!(
        app.borrow().certificates_loading(),
        "and it is looking - rather than showing a table that never fills"
    );

    // The answer lands the same way the menu path's does.
    let now = 1_800_000_000;
    app.borrow_mut()
        .apply_certificates_result_for_test(sample_certificates(now));
    sync_ui(&ui, &app.borrow());

    assert!(
        !app.borrow().certificate_rows().is_empty(),
        "the rows arrive: {}",
        app.borrow().certificates_summary_line()
    );
}

#[test]
fn the_certificates_tool_pops_out_and_docks_back_with_its_own_table() {
    let _serial = serially();
    let root = scratch("pop-out");
    ensure_service();
    init_backend();
    let app = Rc::new(RefCell::new(App::new(root.clone())));
    let ui = MainWindow::new().expect("the window should build");
    gui::wire_callbacks(&ui, &app);
    sync_ui(&ui, &app.borrow());
    ui.show().expect("the window should show");
    ui.window()
        .dispatch_event(WindowEvent::WindowActiveChanged(true));
    let windows = Rc::new(RefCell::new(PaneWindows::new(ui)));
    gui::wire_pop_out(windows.borrow().main(), None, &windows, &app);

    open_certificates_from_the_menu(windows.borrow().main());
    app.borrow_mut()
        .apply_certificates_result_for_test(sample_certificates(now_epoch_seconds()));
    for ui in windows.borrow().windows() {
        sync_ui(ui, &app.borrow());
    }

    // A strong handle, gotten and dropped out of `windows`'s borrow in one
    // statement, so invoking the callback below - which re-enters
    // `windows.borrow_mut()` to record the new popped-out window - does
    // not find it already borrowed (the same reason `pop_out_windows.rs`'s
    // own `pane_handle`/`main_handle` exist).
    let main = windows
        .borrow()
        .main()
        .as_weak()
        .upgrade()
        .expect("the main window is still open");
    main.invoke_pop_out_requested(2);
    for ui in windows.borrow().windows() {
        sync_ui(ui, &app.borrow());
    }

    assert!(windows.borrow().is_popped_out(gui::app::Pane::File));
    let popped = {
        let borrowed = windows.borrow();
        borrowed
            .window_for(gui::app::Pane::File)
            .as_weak()
            .upgrade()
            .expect("the popped window is still open")
    };
    assert!(
        popped.get_tool_showing_certificates(),
        "the popped-out window shows the Certificates tool"
    );
    assert_eq!(popped.get_certificate_rows().row_count(), 3);

    popped.invoke_dock_requested(2);
    for ui in windows.borrow().windows() {
        sync_ui(ui, &app.borrow());
    }

    assert!(!windows.borrow().is_popped_out(gui::app::Pane::File));
    assert!(windows.borrow().main().get_tool_showing_certificates());
    assert_eq!(
        windows.borrow().main().get_certificate_rows().row_count(),
        3,
        "docked back, the main window shows the same table"
    );
}
