//! The file browser: one design for every provider.
//!
//! It only knows [`Entry`], [`Capabilities`] and the file commands, and
//! goes through the daemon's rc-backed file operations, so it works even
//! when the account is unmounted. An action the account cannot do is not
//! shown.
//!
//! ```text
//! AdwNavigationSplitView
//!  ├─ sidebar: accounts (provider icon, name, quota bar)
//!  └─ content: header (up, path, filter, new folder, upload, menu)
//!              list (GtkColumnView) or grid (GtkGridView), remembered
//! ```

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;

use adw::prelude::*;
use gettextrs::gettext;
use gtk::{gdk, gio, glib};

use nuage_core::{
    AccountId, AccountStatus, Badge, Capabilities, Command, Entry, Error, ExclusionRule,
    RemotePath, Reply,
};

use crate::window::{Ui, human_size, open_folder};

/// How often a folder with uploads in progress is listed again.
const UPLOAD_REFRESH: std::time::Duration = std::time::Duration::from_secs(3);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ViewMode {
    List,
    Grid,
}

impl ViewMode {
    fn key(self) -> &'static str {
        match self {
            ViewMode::List => "list",
            ViewMode::Grid => "grid",
        }
    }
}

/// Items cut or copied, waiting for "Paste".
#[derive(Debug, Clone)]
struct Clipboard {
    items: Vec<RemotePath>,
    cut: bool,
}

#[derive(Debug, Default)]
struct State {
    account: Option<AccountId>,
    /// Folder shown, relative to the account root.
    path: String,
    clipboard: Option<Clipboard>,
    /// Incremented by every listing: older answers are dropped.
    generation: u64,
    /// Operations in progress (the header spinner shows).
    busy: u32,
}

/// The browser window.
#[derive(Debug)]
pub struct Browser {
    ui: Rc<Ui>,
    pub window: adw::ApplicationWindow,
    split: adw::NavigationSplitView,
    toasts: adw::ToastOverlay,
    sidebar: gtk::ListBox,
    content_page: adw::NavigationPage,
    breadcrumb: gtk::Box,
    spinner: gtk::Spinner,
    stack: gtk::Stack,
    store: gio::ListStore,
    filter: gtk::CustomFilter,
    search_bar: gtk::SearchBar,
    search: gtk::SearchEntry,
    selection: gtk::MultiSelection,
    column_view: gtk::ColumnView,
    grid: gtk::GridView,
    popover: gtk::PopoverMenu,
    actions: gio::SimpleActionGroup,
    empty_page: adw::StatusPage,
    error_page: adw::StatusPage,
    mode: Cell<ViewMode>,
    state: RefCell<State>,
}

fn entry_of(obj: &glib::Object) -> Option<Entry> {
    obj.downcast_ref::<glib::BoxedAnyObject>()
        .map(|b| b.borrow::<Entry>().clone())
}

/// `a/b/c` → `a/b`.
fn parent(path: &str) -> &str {
    path.rsplit_once('/').map_or("", |(p, _)| p)
}

fn join(dir: &str, name: &str) -> String {
    if dir.is_empty() {
        name.to_owned()
    } else {
        format!("{dir}/{name}")
    }
}

/// `report.odt` → `report (copy).odt`.
fn copy_name(name: &str) -> String {
    match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => format!("{stem} ({}).{ext}", gettext("copy")),
        _ => format!("{name} ({})", gettext("copy")),
    }
}

fn icon_for(entry: &Entry) -> gio::Icon {
    if entry.is_dir() {
        return gio::ThemedIcon::new("folder").upcast();
    }
    if entry.native_doc.is_some() {
        return gio::ThemedIcon::new("x-office-document").upcast();
    }
    let content_type = entry.mime.clone().unwrap_or_else(|| {
        gio::content_type_guess(Some(&entry.name), None)
            .0
            .to_string()
    });
    gio::content_type_get_icon(&content_type)
}

/// Text of the state column (and of the grid's tooltips).
fn state_label(entry: &Entry) -> String {
    let mut parts = Vec::new();
    if entry.has(Badge::Uploading) {
        parts.push(gettext("Uploading"));
    }
    if entry.has(Badge::NotSynced) {
        parts.push(gettext("Not synced"));
    }
    if entry.has(Badge::OnlineOnly) {
        parts.push(gettext("Online only"));
    }
    if parts.is_empty() && entry.native_doc.is_some() {
        parts.push(gettext("Web document"));
    }
    parts.join(" · ")
}

/// Excluded, or offline without a local copy: shown greyed.
fn dimmed(entry: &Entry) -> bool {
    entry.has(Badge::NotSynced) || entry.has(Badge::OnlineOnly)
}

fn modified_text(entry: &Entry) -> String {
    entry
        .modified
        .and_then(|t| glib::DateTime::from_unix_utc(t.timestamp()).ok())
        .and_then(|t| t.to_local().ok())
        .and_then(|t| t.format("%x %H:%M").ok())
        .map(|s| s.to_string())
        .unwrap_or_default()
}

/// Folders first, then `key`.
fn compare(
    a: &glib::Object,
    b: &glib::Object,
    key: fn(&Entry, &Entry) -> std::cmp::Ordering,
) -> gtk::Ordering {
    match (entry_of(a), entry_of(b)) {
        (Some(a), Some(b)) => b.is_dir().cmp(&a.is_dir()).then_with(|| key(&a, &b)).into(),
        _ => gtk::Ordering::Equal,
    }
}

fn by_name(a: &Entry, b: &Entry) -> std::cmp::Ordering {
    a.display_name()
        .to_lowercase()
        .cmp(&b.display_name().to_lowercase())
}

/// Remembered between launches.
fn settings_file() -> PathBuf {
    glib::user_config_dir().join("nuage").join("browser.ini")
}

fn load_mode() -> ViewMode {
    let file = glib::KeyFile::new();
    if file
        .load_from_file(settings_file(), glib::KeyFileFlags::NONE)
        .is_ok()
        && file.string("browser", "view").is_ok_and(|v| v == "grid")
    {
        ViewMode::Grid
    } else {
        ViewMode::List
    }
}

fn save_mode(mode: ViewMode) {
    let file = glib::KeyFile::new();
    file.set_string("browser", "view", mode.key());
    let path = settings_file();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Err(e) = file.save_to_file(&path) {
        tracing::debug!(error = %e, "cannot save the browser settings");
    }
}

impl Browser {
    pub fn new(ui: &Rc<Ui>) -> Rc<Self> {
        let toasts = adw::ToastOverlay::new();
        let split = adw::NavigationSplitView::builder()
            .min_sidebar_width(200.0)
            .build();
        toasts.set_child(Some(&split));
        let window = adw::ApplicationWindow::builder()
            .application(&ui.app)
            .title(gettext("Nuage Files"))
            .default_width(960)
            .default_height(640)
            .content(&toasts)
            .hide_on_close(true)
            .build();

        let search = gtk::SearchEntry::builder()
            .placeholder_text(gettext("Filter this folder"))
            .build();
        let filter = {
            let search = search.clone();
            gtk::CustomFilter::new(move |obj| {
                let text = search.text().to_lowercase();
                text.is_empty()
                    || entry_of(obj)
                        .is_some_and(|e| e.display_name().to_lowercase().contains(&text))
            })
        };
        let store = gio::ListStore::new::<glib::BoxedAnyObject>();
        let filtered = gtk::FilterListModel::new(Some(store.clone()), Some(filter.clone()));
        let column_view = gtk::ColumnView::builder()
            .show_row_separators(false)
            .css_classes(["data-table"])
            .build();
        let sorted = gtk::SortListModel::new(Some(filtered), column_view.sorter());
        let selection = gtk::MultiSelection::new(Some(sorted));
        column_view.set_model(Some(&selection));
        let grid = gtk::GridView::builder()
            .model(&selection)
            .max_columns(12)
            .min_columns(2)
            .enable_rubberband(true)
            .build();

        let stack = gtk::Stack::new();
        let popover = gtk::PopoverMenu::from_model(None::<&gio::MenuModel>);
        popover.set_has_arrow(false);
        popover.set_halign(gtk::Align::Start);
        popover.set_parent(&stack);

        let this = Rc::new(Self {
            ui: Rc::clone(ui),
            window,
            split,
            toasts,
            sidebar: gtk::ListBox::builder()
                .css_classes(["navigation-sidebar"])
                .build(),
            content_page: adw::NavigationPage::builder()
                .title(gettext("Files"))
                .tag("content")
                .build(),
            breadcrumb: gtk::Box::new(gtk::Orientation::Horizontal, 0),
            spinner: gtk::Spinner::new(),
            stack,
            store,
            filter,
            search_bar: gtk::SearchBar::new(),
            search,
            selection,
            column_view,
            grid,
            popover,
            actions: gio::SimpleActionGroup::new(),
            empty_page: adw::StatusPage::builder()
                .icon_name("folder-symbolic")
                .title(gettext("Folder Is Empty"))
                .description(gettext("Drop files here to upload them."))
                .build(),
            error_page: adw::StatusPage::builder()
                .icon_name("dialog-warning-symbolic")
                .title(gettext("Cannot Open This Folder"))
                .build(),
            mode: Cell::new(load_mode()),
            state: RefCell::new(State::default()),
        });
        this.build();
        this
    }

    fn build(self: &Rc<Self>) {
        self.build_sidebar();
        self.build_views();
        self.build_content();
        self.build_actions();
        self.update_accounts();
        self.show_placeholder();
    }

    fn build_sidebar(self: &Rc<Self>) {
        let header = adw::HeaderBar::new();
        let view = adw::ToolbarView::new();
        view.add_top_bar(&header);
        let scrolled = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .child(&self.sidebar)
            .build();
        view.set_content(Some(&scrolled));
        self.split.set_sidebar(Some(
            &adw::NavigationPage::builder()
                .title(gettext("Accounts"))
                .child(&view)
                .build(),
        ));
        let this = Rc::clone(self);
        self.sidebar.connect_row_activated(move |_, row| {
            let accounts = this.ui.accounts();
            if let Some(status) = usize::try_from(row.index())
                .ok()
                .and_then(|i| accounts.get(i))
            {
                this.navigate(&status.account.id, "");
                this.split.set_show_content(true);
            }
        });
    }

    /// The list and grid views, sharing one model and selection.
    fn build_views(self: &Rc<Self>) {
        let name = self.column(&gettext("Name"), true, |entry, cell| {
            let row = cell.downcast_ref::<gtk::Box>().expect("name cell");
            let icon = row
                .first_child()
                .and_downcast::<gtk::Image>()
                .expect("icon");
            let label = icon
                .next_sibling()
                .and_downcast::<gtk::Label>()
                .expect("label");
            icon.set_from_gicon(&icon_for(entry));
            label.set_text(entry.display_name());
            row.set_opacity(if dimmed(entry) { 0.55 } else { 1.0 });
        });
        name.set_sorter(Some(&gtk::CustomSorter::new(|a, b| compare(a, b, by_name))));
        let size = self.column(&gettext("Size"), false, |entry, cell| {
            let label = cell.downcast_ref::<gtk::Label>().expect("label");
            label.set_text(&entry.size.map(human_size).unwrap_or_default());
        });
        size.set_sorter(Some(&gtk::CustomSorter::new(|a, b| {
            compare(a, b, |a, b| a.size.cmp(&b.size).then_with(|| by_name(a, b)))
        })));
        let modified = self.column(&gettext("Modified"), false, |entry, cell| {
            let label = cell.downcast_ref::<gtk::Label>().expect("label");
            label.set_text(&modified_text(entry));
        });
        modified.set_sorter(Some(&gtk::CustomSorter::new(|a, b| {
            compare(a, b, |a, b| {
                a.modified.cmp(&b.modified).then_with(|| by_name(a, b))
            })
        })));
        let state = self.column(&gettext("State"), false, |entry, cell| {
            let label = cell.downcast_ref::<gtk::Label>().expect("label");
            label.set_text(&state_label(entry));
        });
        for column in [&name, &size, &modified, &state] {
            self.column_view.append_column(column);
        }

        let factory = gtk::SignalListItemFactory::new();
        {
            let this = Rc::clone(self);
            factory.connect_setup(move |_, item| {
                let item = item.downcast_ref::<gtk::ListItem>().expect("list item");
                let tile = gtk::Box::builder()
                    .orientation(gtk::Orientation::Vertical)
                    .spacing(6)
                    .margin_top(6)
                    .margin_bottom(6)
                    .width_request(104)
                    .build();
                tile.append(&gtk::Image::builder().pixel_size(64).build());
                tile.append(
                    &gtk::Label::builder()
                        .wrap(true)
                        .wrap_mode(gtk::pango::WrapMode::WordChar)
                        .lines(2)
                        .ellipsize(gtk::pango::EllipsizeMode::End)
                        .justify(gtk::Justification::Center)
                        .max_width_chars(14)
                        .build(),
                );
                this.attach_item_menu(tile.upcast_ref(), item);
                item.set_child(Some(&tile));
            });
        }
        factory.connect_bind(|_, item| {
            let item = item.downcast_ref::<gtk::ListItem>().expect("list item");
            let (Some(entry), Some(tile)) = (
                item.item().as_ref().and_then(entry_of),
                item.child().and_downcast::<gtk::Box>(),
            ) else {
                return;
            };
            let icon = tile
                .first_child()
                .and_downcast::<gtk::Image>()
                .expect("icon");
            let label = icon
                .next_sibling()
                .and_downcast::<gtk::Label>()
                .expect("label");
            icon.set_from_gicon(&icon_for(&entry));
            label.set_text(entry.display_name());
            let state = state_label(&entry);
            tile.set_tooltip_text((!state.is_empty()).then_some(state.as_str()));
            tile.set_opacity(if dimmed(&entry) { 0.55 } else { 1.0 });
        });
        self.grid.set_factory(Some(&factory));

        let this = Rc::clone(self);
        self.column_view
            .connect_activate(move |_, position| this.activate(position));
        let this = Rc::clone(self);
        self.grid
            .connect_activate(move |_, position| this.activate(position));
        let this = Rc::clone(self);
        self.selection
            .connect_selection_changed(move |_, _, _| this.update_actions());
    }

    /// A list column whose cells show `bind(entry, cell)`.
    fn column(
        self: &Rc<Self>,
        title: &str,
        is_name: bool,
        bind: fn(&Entry, &gtk::Widget),
    ) -> gtk::ColumnViewColumn {
        let factory = gtk::SignalListItemFactory::new();
        let this = Rc::clone(self);
        factory.connect_setup(move |_, item| {
            let item = item.downcast_ref::<gtk::ListItem>().expect("list item");
            let cell: gtk::Widget = if is_name {
                let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
                row.append(&gtk::Image::new());
                row.append(
                    &gtk::Label::builder()
                        .xalign(0.0)
                        .hexpand(true)
                        .ellipsize(gtk::pango::EllipsizeMode::Middle)
                        .build(),
                );
                row.upcast()
            } else {
                gtk::Label::builder()
                    .xalign(0.0)
                    .css_classes(["dim-label", "numeric"])
                    .build()
                    .upcast()
            };
            this.attach_item_menu(&cell, item);
            item.set_child(Some(&cell));
        });
        factory.connect_bind(move |_, item| {
            let item = item.downcast_ref::<gtk::ListItem>().expect("list item");
            if let (Some(entry), Some(cell)) =
                (item.item().as_ref().and_then(entry_of), item.child())
            {
                bind(&entry, &cell);
            }
        });
        gtk::ColumnViewColumn::builder()
            .title(title)
            .factory(&factory)
            .expand(is_name)
            .resizable(true)
            .build()
    }

    /// Right click on an item: select it (unless it is selected already)
    /// and show what can be done with the selection.
    fn attach_item_menu(self: &Rc<Self>, widget: &gtk::Widget, item: &gtk::ListItem) {
        let gesture = gtk::GestureClick::builder()
            .button(gdk::BUTTON_SECONDARY)
            .build();
        let this = Rc::clone(self);
        let item = item.downgrade();
        gesture.connect_pressed(move |gesture, _, x, y| {
            let Some(item) = item.upgrade() else { return };
            let position = item.position();
            if position == gtk::INVALID_LIST_POSITION {
                return;
            }
            gesture.set_state(gtk::EventSequenceState::Claimed);
            if !this.selection.is_selected(position) {
                this.selection.select_item(position, true);
            }
            if let Some(widget) = gesture.widget() {
                this.popup_menu(&widget, x, y, true);
            }
        });
        widget.add_controller(gesture);
    }

    fn build_content(self: &Rc<Self>) {
        let header = adw::HeaderBar::new();
        let up = gtk::Button::builder()
            .icon_name("go-up-symbolic")
            .tooltip_text(gettext("Parent Folder"))
            .action_name("browser.go-up")
            .build();
        header.pack_start(&up);
        let breadcrumb = gtk::ScrolledWindow::builder()
            .vscrollbar_policy(gtk::PolicyType::Never)
            .propagate_natural_width(true)
            .child(&self.breadcrumb)
            .build();
        header.set_title_widget(Some(&breadcrumb));

        let menu = gio::Menu::new();
        let view_section = gio::Menu::new();
        view_section.append(Some(&gettext("_List")), Some("browser.view::list"));
        view_section.append(Some(&gettext("_Grid")), Some("browser.view::grid"));
        menu.append_section(None, &view_section);
        let folder_section = gio::Menu::new();
        folder_section.append(Some(&gettext("_Refresh")), Some("browser.refresh"));
        folder_section.append(
            Some(&gettext("Show in _Files")),
            Some("browser.show-folder"),
        );
        folder_section.append(Some(&gettext("_Paste")), Some("browser.paste"));
        menu.append_section(None, &folder_section);
        header.pack_end(
            &gtk::MenuButton::builder()
                .icon_name("view-more-symbolic")
                .menu_model(&menu)
                .tooltip_text(gettext("Folder Menu"))
                .build(),
        );
        header.pack_end(
            &gtk::Button::builder()
                .icon_name("document-send-symbolic")
                .tooltip_text(gettext("Upload Files"))
                .action_name("browser.upload")
                .build(),
        );
        header.pack_end(
            &gtk::Button::builder()
                .icon_name("folder-new-symbolic")
                .tooltip_text(gettext("New Folder"))
                .action_name("browser.new-folder")
                .build(),
        );
        let search_button = gtk::ToggleButton::builder()
            .icon_name("system-search-symbolic")
            .tooltip_text(gettext("Filter"))
            .build();
        search_button
            .bind_property("active", &self.search_bar, "search-mode-enabled")
            .bidirectional()
            .build();
        header.pack_end(&search_button);
        header.pack_end(&self.spinner);

        self.search_bar.set_child(Some(&self.search));
        self.search_bar.connect_entry(&self.search);
        self.search_bar.set_key_capture_widget(Some(&self.window));
        {
            let filter = self.filter.clone();
            self.search
                .connect_search_changed(move |_| filter.changed(gtk::FilterChange::Different));
        }

        let list_scroll = gtk::ScrolledWindow::builder()
            .child(&self.column_view)
            .vexpand(true)
            .build();
        let grid_scroll = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .child(&self.grid)
            .vexpand(true)
            .build();
        self.stack.add_named(&list_scroll, Some("list"));
        self.stack.add_named(&grid_scroll, Some("grid"));
        self.stack.add_named(&self.empty_page, Some("empty"));
        self.stack.add_named(&self.error_page, Some("error"));
        self.stack.add_named(
            &adw::StatusPage::builder()
                .icon_name("folder-remote-symbolic")
                .title(gettext("Choose an Account"))
                .build(),
            Some("placeholder"),
        );
        let loading = gtk::Spinner::builder()
            .spinning(true)
            .width_request(32)
            .height_request(32)
            .halign(gtk::Align::Center)
            .valign(gtk::Align::Center)
            .build();
        self.stack.add_named(&loading, Some("loading"));

        // Right click on the background: the folder's menu.
        let gesture = gtk::GestureClick::builder()
            .button(gdk::BUTTON_SECONDARY)
            .build();
        {
            let this = Rc::clone(self);
            gesture.connect_pressed(move |gesture, _, x, y| {
                if this.state.borrow().account.is_none() {
                    return;
                }
                this.selection.unselect_all();
                if let Some(widget) = gesture.widget() {
                    this.popup_menu(&widget, x, y, false);
                }
            });
        }
        self.stack.add_controller(gesture);

        // Files dropped from the file manager are uploaded here.
        let drop = gtk::DropTarget::new(gdk::FileList::static_type(), gdk::DragAction::COPY);
        {
            let this = Rc::clone(self);
            drop.connect_drop(move |_, value, _, _| {
                let Ok(files) = value.get::<gdk::FileList>() else {
                    return false;
                };
                let paths: Vec<PathBuf> = files.files().iter().filter_map(|f| f.path()).collect();
                if paths.is_empty() || this.state.borrow().account.is_none() {
                    return false;
                }
                this.upload(paths);
                true
            });
        }
        self.stack.add_controller(drop);

        let view = adw::ToolbarView::new();
        view.add_top_bar(&header);
        view.add_top_bar(&self.search_bar);
        view.set_content(Some(&self.stack));
        self.content_page.set_child(Some(&view));
        self.split.set_content(Some(&self.content_page));
    }

    fn build_actions(self: &Rc<Self>) {
        type Handler = fn(&Rc<Browser>);
        let simple: [(&str, Handler); 17] = [
            ("open", |b| b.open_selected()),
            ("download", |b| b.download_selected()),
            ("rename", |b| b.rename_selected()),
            ("cut", |b| b.clip(true)),
            ("copy", |b| b.clip(false)),
            ("paste", |b| b.paste()),
            ("delete", |b| b.delete_selected()),
            ("copy-link", |b| b.copy_link()),
            ("properties", |b| b.properties()),
            ("show-in-files", |b| b.show_in_files()),
            ("show-folder", |b| b.show_folder()),
            ("exclude", |b| b.set_excluded(true)),
            ("include", |b| b.set_excluded(false)),
            ("new-folder", |b| b.new_folder()),
            ("upload", |b| b.choose_uploads()),
            ("refresh", |b| b.load()),
            ("go-up", |b| b.go_up()),
        ];
        for (name, handler) in simple {
            let action = gio::SimpleAction::new(name, None);
            let this = Rc::clone(self);
            action.connect_activate(move |_, _| handler(&this));
            self.actions.add_action(&action);
        }
        let view = gio::SimpleAction::new_stateful(
            "view",
            Some(glib::VariantTy::STRING),
            &self.mode.get().key().to_variant(),
        );
        {
            let this = Rc::clone(self);
            view.connect_activate(move |action, value| {
                let Some(key) = value.and_then(|v| v.get::<String>()) else {
                    return;
                };
                action.set_state(&key.to_variant());
                let mode = if key == "grid" {
                    ViewMode::Grid
                } else {
                    ViewMode::List
                };
                this.mode.set(mode);
                save_mode(mode);
                if matches!(
                    this.stack.visible_child_name().as_deref(),
                    Some("list" | "grid")
                ) {
                    this.stack.set_visible_child_name(mode.key());
                }
            });
        }
        self.actions.add_action(&view);
        let search = gio::SimpleAction::new("search", None);
        {
            let bar = self.search_bar.clone();
            search.connect_activate(move |_, _| bar.set_search_mode(!bar.is_search_mode()));
        }
        self.actions.add_action(&search);
        self.window
            .insert_action_group("browser", Some(&self.actions));

        let app = &self.ui.app;
        for (action, accels) in [
            ("browser.delete", &["Delete"][..]),
            ("browser.rename", &["F2"]),
            ("browser.cut", &["<Control>x"]),
            ("browser.copy", &["<Control>c"]),
            ("browser.paste", &["<Control>v"]),
            ("browser.refresh", &["F5", "<Control>r"]),
            ("browser.go-up", &["<Alt>Up"]),
            ("browser.new-folder", &["<Control><Shift>n"]),
            ("browser.search", &["<Control>f"]),
            ("browser.properties", &["<Alt>Return"]),
        ] {
            app.set_accels_for_action(action, accels);
        }
        self.update_actions();
    }

    fn set_enabled(&self, name: &str, enabled: bool) {
        if let Some(action) = self
            .actions
            .lookup_action(name)
            .and_downcast::<gio::SimpleAction>()
        {
            action.set_enabled(enabled);
        }
    }

    // State

    fn account(&self) -> Option<AccountStatus> {
        let id = self.state.borrow().account.clone()?;
        self.ui.accounts().into_iter().find(|a| a.account.id == id)
    }

    fn capabilities(&self) -> Capabilities {
        let Some(account) = self.account() else {
            return Capabilities::default();
        };
        self.ui
            .providers()
            .into_iter()
            .find(|p| p.id == account.account.provider_id)
            .map(|p| p.capabilities)
            .unwrap_or_default()
    }

    fn remote(&self, path: &str) -> Option<RemotePath> {
        let id = self.state.borrow().account.clone()?;
        Some(RemotePath::new(id, path))
    }

    fn selected(&self) -> Vec<Entry> {
        let model = &self.selection;
        (0..model.n_items())
            .filter(|&i| model.is_selected(i))
            .filter_map(|i| model.item(i).as_ref().and_then(entry_of))
            .collect()
    }

    /// The selected entry, when exactly one is.
    fn single(&self) -> Option<Entry> {
        let mut selected = self.selected();
        (selected.len() == 1).then(|| selected.remove(0))
    }

    fn update_actions(&self) {
        let selected = self.selected();
        let one = selected.len() == 1;
        let any = !selected.is_empty();
        let has_account = self.state.borrow().account.is_some();
        let mounted = self.account().is_some_and(|a| a.mounted);
        let caps = self.capabilities();
        let excluded = selected.first().is_some_and(|e| e.has(Badge::NotSynced));
        let at_root = self.state.borrow().path.is_empty();
        self.set_enabled("open", one);
        self.set_enabled("download", any);
        self.set_enabled("rename", one);
        self.set_enabled("cut", any);
        self.set_enabled("copy", any);
        self.set_enabled(
            "paste",
            has_account && self.state.borrow().clipboard.is_some(),
        );
        self.set_enabled("delete", any);
        self.set_enabled("copy-link", one && caps.public_link);
        self.set_enabled("properties", one);
        self.set_enabled("show-in-files", one && mounted && !excluded);
        self.set_enabled("show-folder", has_account && mounted);
        self.set_enabled("exclude", one && !excluded);
        self.set_enabled("include", one && excluded);
        self.set_enabled("new-folder", has_account);
        self.set_enabled("upload", has_account);
        self.set_enabled("refresh", has_account);
        self.set_enabled("go-up", has_account && !at_root);
    }

    /// The context menu, with only what can be done here.
    fn popup_menu(&self, widget: &gtk::Widget, x: f64, y: f64, on_item: bool) {
        self.update_actions();
        let menu = gio::Menu::new();
        let section = |items: &[(String, &str, bool)]| {
            let s = gio::Menu::new();
            for (label, action, shown) in items {
                if *shown {
                    s.append(Some(label), Some(action));
                }
            }
            if s.n_items() > 0 {
                menu.append_section(None, &s);
            }
        };
        let mounted = self.account().is_some_and(|a| a.mounted);
        let caps = self.capabilities();
        let clipboard = self.state.borrow().clipboard.is_some();
        if on_item {
            let selected = self.selected();
            let one = selected.len() == 1;
            let excluded = one && selected[0].has(Badge::NotSynced);
            section(&[
                (gettext("_Open"), "browser.open", one),
                (
                    gettext("Show in _Files"),
                    "browser.show-in-files",
                    one && mounted && !excluded,
                ),
            ]);
            section(&[
                (gettext("_Download To…"), "browser.download", true),
                (
                    gettext("Copy _Link"),
                    "browser.copy-link",
                    one && caps.public_link,
                ),
            ]);
            section(&[
                (gettext("Cu_t"), "browser.cut", true),
                (gettext("_Copy"), "browser.copy", true),
            ]);
            section(&[
                (gettext("_Rename…"), "browser.rename", one),
                (
                    gettext("_Exclude From Sync"),
                    "browser.exclude",
                    one && !excluded,
                ),
                (gettext("_Include in Sync"), "browser.include", excluded),
            ]);
            let delete = if caps.trash {
                gettext("Move to _Trash")
            } else {
                gettext("_Delete…")
            };
            section(&[(delete, "browser.delete", true)]);
            section(&[(gettext("_Properties"), "browser.properties", one)]);
        } else {
            section(&[
                (gettext("New _Folder…"), "browser.new-folder", true),
                (gettext("_Upload Files…"), "browser.upload", true),
                (gettext("_Paste"), "browser.paste", clipboard),
            ]);
            section(&[
                (gettext("_Refresh"), "browser.refresh", true),
                (gettext("Show in _Files"), "browser.show-folder", mounted),
            ]);
        }
        let Some(point) =
            widget.compute_point(&self.stack, &gtk::graphene::Point::new(x as f32, y as f32))
        else {
            return;
        };
        self.popover.set_menu_model(Some(&menu));
        self.popover.set_pointing_to(Some(&gdk::Rectangle::new(
            point.x() as i32,
            point.y() as i32,
            1,
            1,
        )));
        self.popover.popup();
    }

    // Accounts and navigation

    /// Shows the accounts again (after a status change).
    pub fn update_accounts(self: &Rc<Self>) {
        while let Some(child) = self.sidebar.first_child() {
            self.sidebar.remove(&child);
        }
        let accounts = self.ui.accounts();
        let providers = self.ui.providers();
        let current = self.state.borrow().account.clone();
        for status in &accounts {
            let icon = providers
                .iter()
                .find(|p| p.id == status.account.provider_id)
                .map_or("folder-remote-symbolic", |p| p.icon_name.as_str());
            let row = gtk::Box::builder()
                .orientation(gtk::Orientation::Horizontal)
                .spacing(10)
                .margin_top(6)
                .margin_bottom(6)
                .build();
            row.append(&gtk::Image::from_icon_name(icon));
            let text = gtk::Box::new(gtk::Orientation::Vertical, 4);
            text.append(
                &gtk::Label::builder()
                    .label(&status.account.display_name)
                    .xalign(0.0)
                    .ellipsize(gtk::pango::EllipsizeMode::End)
                    .build(),
            );
            if let Some(percent) = status.quota.and_then(|q| q.used_percent()) {
                let bar = gtk::LevelBar::builder()
                    .min_value(0.0)
                    .max_value(100.0)
                    .value(percent)
                    .build();
                if let Some(q) = status.quota
                    && let (Some(used), Some(total)) = (q.used, q.total)
                {
                    bar.set_tooltip_text(Some(
                        &gettext("{used} of {total} used")
                            .replace("{used}", &human_size(used))
                            .replace("{total}", &human_size(total)),
                    ));
                }
                text.append(&bar);
            }
            text.set_hexpand(true);
            row.append(&text);
            self.sidebar.append(&row);
            if current.as_ref() == Some(&status.account.id)
                && let Some(r) = self.sidebar.last_child().and_downcast::<gtk::ListBoxRow>()
            {
                self.sidebar.select_row(Some(&r));
            }
        }
        // The account shown was removed.
        if let Some(id) = current
            && !accounts.iter().any(|a| a.account.id == id)
        {
            self.state.borrow_mut().account = None;
            self.store.remove_all();
            self.show_placeholder();
        }
        self.update_actions();
    }

    fn show_placeholder(&self) {
        self.stack.set_visible_child_name("placeholder");
        self.content_page.set_title(&gettext("Files"));
        while let Some(child) = self.breadcrumb.first_child() {
            self.breadcrumb.remove(&child);
        }
    }

    /// Shows a folder of an account.
    pub fn navigate(self: &Rc<Self>, account: &AccountId, path: &str) {
        {
            let mut st = self.state.borrow_mut();
            st.account = Some(account.clone());
            st.path = path.to_owned();
        }
        self.search.set_text("");
        self.search_bar.set_search_mode(false);
        self.update_breadcrumb();
        let accounts = self.ui.accounts();
        if let Some(i) = accounts.iter().position(|a| &a.account.id == account)
            && let Some(row) = self.sidebar.row_at_index(i as i32)
        {
            self.sidebar.select_row(Some(&row));
        }
        self.load();
    }

    fn go_up(self: &Rc<Self>) {
        let (account, path) = {
            let st = self.state.borrow();
            (st.account.clone(), st.path.clone())
        };
        if let Some(account) = account
            && !path.is_empty()
        {
            self.navigate(&account, parent(&path));
        }
    }

    fn update_breadcrumb(self: &Rc<Self>) {
        while let Some(child) = self.breadcrumb.first_child() {
            self.breadcrumb.remove(&child);
        }
        let Some(status) = self.account() else { return };
        let path = self.state.borrow().path.clone();
        let mut crumbs = vec![(status.account.display_name.clone(), String::new())];
        let mut acc = String::new();
        for name in path.split('/').filter(|n| !n.is_empty()) {
            acc = join(&acc, name);
            crumbs.push((name.to_owned(), acc.clone()));
        }
        self.content_page
            .set_title(&crumbs.last().map(|c| c.0.clone()).unwrap_or_default());
        let last = crumbs.len() - 1;
        for (i, (label, target)) in crumbs.into_iter().enumerate() {
            if i > 0 {
                self.breadcrumb.append(
                    &gtk::Label::builder()
                        .label("/")
                        .css_classes(["dim-label"])
                        .build(),
                );
            }
            let button = gtk::Button::builder()
                .label(&label)
                .css_classes(["flat"])
                .build();
            if i == last {
                button.add_css_class("heading");
            }
            let this = Rc::clone(self);
            let id = status.account.id.clone();
            button.connect_clicked(move |_| this.navigate(&id, &target));
            self.breadcrumb.append(&button);
        }
    }

    /// Lists the current folder again.
    fn load(self: &Rc<Self>) {
        let Some(at) = self.remote(&self.state.borrow().path.clone()) else {
            return;
        };
        let generation = {
            let mut st = self.state.borrow_mut();
            st.generation += 1;
            st.generation
        };
        if self.store.n_items() == 0 {
            self.stack.set_visible_child_name("loading");
        }
        self.update_actions();
        let this = Rc::clone(self);
        glib::spawn_future_local(async move {
            let answer = this
                .ui
                .backend
                .call(Command::List { path: at.clone() })
                .await;
            if this.state.borrow().generation != generation {
                return;
            }
            match answer {
                Ok(Reply::Entries(entries)) => {
                    let uploading = entries.iter().any(|e| e.has(Badge::Uploading));
                    let objects: Vec<glib::BoxedAnyObject> =
                        entries.into_iter().map(glib::BoxedAnyObject::new).collect();
                    this.store.splice(0, this.store.n_items(), &objects);
                    this.stack.set_visible_child_name(if objects.is_empty() {
                        "empty"
                    } else {
                        this.mode.get().key()
                    });
                    this.update_actions();
                    if uploading {
                        let this = Rc::clone(&this);
                        glib::timeout_add_local_once(UPLOAD_REFRESH, move || {
                            if this.state.borrow().generation == generation
                                && this.window.is_visible()
                            {
                                this.load();
                            }
                        });
                    }
                }
                Ok(_) => {}
                Err(message) => {
                    this.store.remove_all();
                    this.error_page.set_description(Some(&message));
                    this.stack.set_visible_child_name("error");
                }
            }
        });
    }

    fn activate(self: &Rc<Self>, position: u32) {
        if let Some(entry) = self.selection.item(position).as_ref().and_then(entry_of) {
            self.open(entry);
        }
    }

    fn open_selected(self: &Rc<Self>) {
        if let [entry] = self.selected().as_slice() {
            self.open(entry.clone());
        }
    }

    /// Folders open in the browser; Google Docs in the web browser; files
    /// with the default application, from the mounted folder or else from
    /// a downloaded copy.
    fn open(self: &Rc<Self>, entry: Entry) {
        let Some(status) = self.account() else { return };
        if entry.is_dir() {
            self.navigate(&status.account.id, &entry.path);
            return;
        }
        if let Some(url) = entry.native_doc.as_ref().and_then(|d| d.web_url.clone()) {
            gtk::UriLauncher::new(&url).launch(
                Some(&self.window),
                gio::Cancellable::NONE,
                |result| {
                    if let Err(e) = result {
                        tracing::warn!(error = %e, "cannot open the document");
                    }
                },
            );
            return;
        }
        if entry.has(Badge::OnlineOnly) {
            self.toast(&gettext(
                "This file is not on this computer. It can be opened once the connection is back.",
            ));
            return;
        }
        if status.mounted && !entry.has(Badge::NotSynced) {
            self.launch(status.account.mount_point.join(&entry.path));
            return;
        }
        // Not in the mounted folder: open a downloaded copy.
        let dir = glib::user_cache_dir()
            .join("nuage")
            .join("open")
            .join(glib::uuid_string_random().as_str());
        if let Err(e) = std::fs::create_dir_all(&dir) {
            self.toast(&e.to_string());
            return;
        }
        let local = dir.join(&entry.name);
        let from = RemotePath::new(status.account.id.clone(), &entry.path);
        let this = Rc::clone(self);
        self.run(Command::Download { from, local: dir }, move |_| {
            this.launch(local)
        });
    }

    fn launch(&self, path: PathBuf) {
        let toasts = self.toasts.clone();
        gtk::FileLauncher::new(Some(&gio::File::for_path(&path))).launch(
            Some(&self.window),
            gio::Cancellable::NONE,
            move |result| {
                if let Err(e) = result
                    && !e.matches(gtk::DialogError::Dismissed)
                {
                    toasts.add_toast(adw::Toast::new(&e.to_string()));
                }
            },
        );
    }

    pub fn toast(&self, text: &str) {
        self.toasts.add_toast(adw::Toast::new(text));
    }

    /// Sends a command with the header spinner turning; `done` runs on
    /// success, the error shows as a toast.
    fn run(self: &Rc<Self>, command: Command, done: impl FnOnce(Reply) + 'static) {
        self.set_busy(1);
        let this = Rc::clone(self);
        glib::spawn_future_local(async move {
            let answer = this.ui.backend.call(command).await;
            this.set_busy(-1);
            match answer {
                Ok(reply) => done(reply),
                Err(message) => this.toast(&message),
            }
        });
    }

    fn set_busy(&self, delta: i32) {
        let busy = {
            let mut st = self.state.borrow_mut();
            st.busy = st.busy.saturating_add_signed(delta);
            st.busy
        };
        self.spinner.set_spinning(busy > 0);
        self.spinner.set_visible(busy > 0);
    }

    /// Runs commands one after the other, then lists the folder again.
    fn run_all(self: &Rc<Self>, commands: Vec<Command>, done_message: String) {
        if commands.is_empty() {
            return;
        }
        self.set_busy(1);
        let this = Rc::clone(self);
        glib::spawn_future_local(async move {
            let mut failed = None;
            for command in commands {
                if let Err(message) = this.ui.backend.call(command).await {
                    failed = Some(message);
                    break;
                }
            }
            this.set_busy(-1);
            this.toast(failed.as_deref().unwrap_or(&done_message));
            this.load();
        });
    }

    // Actions

    fn download_selected(self: &Rc<Self>) {
        let selected = self.selected();
        let Some(id) = self.state.borrow().account.clone() else {
            return;
        };
        if selected.is_empty() {
            return;
        }
        let this = Rc::clone(self);
        glib::spawn_future_local(async move {
            let dialog = gtk::FileDialog::builder()
                .title(gettext("Download To"))
                .modal(true)
                .build();
            let Ok(folder) = dialog.select_folder_future(Some(&this.window)).await else {
                return;
            };
            let Some(local) = folder.path() else { return };
            let n = selected.len();
            let commands = selected
                .iter()
                .map(|e| Command::Download {
                    from: RemotePath::new(id.clone(), &e.path),
                    local: local.clone(),
                })
                .collect();
            this.toast(&gettext("Downloading…"));
            this.run_all(
                commands,
                ngettext_n("Downloaded {} item", "Downloaded {} items", n),
            );
        });
    }

    fn choose_uploads(self: &Rc<Self>) {
        let this = Rc::clone(self);
        glib::spawn_future_local(async move {
            let dialog = gtk::FileDialog::builder()
                .title(gettext("Upload Files"))
                .modal(true)
                .build();
            let Ok(files) = dialog.open_multiple_future(Some(&this.window)).await else {
                return;
            };
            let paths: Vec<PathBuf> = (0..files.n_items())
                .filter_map(|i| files.item(i).and_downcast::<gio::File>())
                .filter_map(|f| f.path())
                .collect();
            this.upload(paths);
        });
    }

    /// Uploads local files or folders into the current folder.
    fn upload(self: &Rc<Self>, paths: Vec<PathBuf>) {
        let Some(to) = self.remote(&self.state.borrow().path.clone()) else {
            return;
        };
        let n = paths.len();
        let commands = paths
            .into_iter()
            .map(|local| Command::Upload {
                local,
                to: to.clone(),
            })
            .collect();
        self.toast(&gettext("Uploading…"));
        self.run_all(
            commands,
            ngettext_n("Uploaded {} item", "Uploaded {} items", n),
        );
    }

    /// Asks for a name; `None` when cancelled.
    async fn ask_name(&self, heading: &str, action: &str, initial: &str) -> Option<String> {
        let entry = gtk::Entry::builder()
            .text(initial)
            .activates_default(true)
            .build();
        let dialog = adw::AlertDialog::builder()
            .heading(heading)
            .extra_child(&entry)
            .build();
        dialog.add_response("cancel", &gettext("_Cancel"));
        dialog.add_response("ok", action);
        dialog.set_response_appearance("ok", adw::ResponseAppearance::Suggested);
        dialog.set_default_response(Some("ok"));
        dialog.set_close_response("cancel");
        {
            let dialog = dialog.clone();
            entry.connect_changed(move |e| {
                let name = e.text();
                let valid = !name.trim().is_empty() && !name.contains('/');
                dialog.set_response_enabled("ok", valid);
            });
        }
        // Select the name without its extension, as Files does.
        let stem = initial
            .rfind('.')
            .filter(|&i| i > 0)
            .unwrap_or(initial.len());
        entry.grab_focus();
        entry.select_region(0, initial[..stem].chars().count() as i32);
        let response = dialog.choose_future(Some(&self.window)).await;
        (response == "ok").then(|| entry.text().trim().to_owned())
    }

    fn new_folder(self: &Rc<Self>) {
        let this = Rc::clone(self);
        glib::spawn_future_local(async move {
            let Some(name) = this
                .ask_name(&gettext("New Folder"), &gettext("_Create"), "")
                .await
            else {
                return;
            };
            let path = join(&this.state.borrow().path.clone(), &name);
            let Some(at) = this.remote(&path) else { return };
            let reload = Rc::clone(&this);
            this.run(Command::Mkdir { path: at }, move |_| reload.load());
        });
    }

    fn rename_selected(self: &Rc<Self>) {
        let Some(entry) = self.single() else {
            return;
        };
        let this = Rc::clone(self);
        glib::spawn_future_local(async move {
            let Some(name) = this
                .ask_name(&gettext("Rename"), &gettext("_Rename"), &entry.name)
                .await
            else {
                return;
            };
            if name == entry.name {
                return;
            }
            let (Some(from), Some(to)) = (
                this.remote(&entry.path),
                this.remote(&join(parent(&entry.path), &name)),
            ) else {
                return;
            };
            let reload = Rc::clone(&this);
            this.run(Command::Move { from, to }, move |_| reload.load());
        });
    }

    fn clip(&self, cut: bool) {
        let items: Vec<RemotePath> = self
            .selected()
            .iter()
            .filter_map(|e| self.remote(&e.path))
            .collect();
        if items.is_empty() {
            return;
        }
        let n = items.len();
        self.state.borrow_mut().clipboard = Some(Clipboard { items, cut });
        self.update_actions();
        self.toast(&if cut {
            ngettext_n(
                "{} item will be moved where you paste it",
                "{} items will be moved where you paste them",
                n,
            )
        } else {
            ngettext_n(
                "{} item will be copied where you paste it",
                "{} items will be copied where you paste them",
                n,
            )
        });
    }

    fn paste(self: &Rc<Self>) {
        let Some(clipboard) = self.state.borrow().clipboard.clone() else {
            return;
        };
        let here = self.state.borrow().path.clone();
        let Some(to) = self.remote(&here) else { return };
        let commands = clipboard
            .items
            .iter()
            .filter_map(|from| {
                let same_place = from.account == to.account && parent(&from.path) == here;
                match (clipboard.cut, same_place) {
                    // Already here.
                    (true, true) => None,
                    (true, false) => Some(Command::Move {
                        from: from.clone(),
                        to: to.clone(),
                    }),
                    // A copy next to the original gets another name.
                    (false, true) => {
                        let name = from.path.rsplit('/').next().unwrap_or(&from.path);
                        Some(Command::Copy {
                            from: from.clone(),
                            to: RemotePath::new(to.account.clone(), join(&here, &copy_name(name))),
                        })
                    }
                    (false, false) => Some(Command::Copy {
                        from: from.clone(),
                        to: to.clone(),
                    }),
                }
            })
            .collect::<Vec<_>>();
        let n = clipboard.items.len();
        if clipboard.cut {
            self.state.borrow_mut().clipboard = None;
            self.update_actions();
        }
        let message = if clipboard.cut {
            ngettext_n("Moved {} item", "Moved {} items", n)
        } else {
            ngettext_n("Copied {} item", "Copied {} items", n)
        };
        self.run_all(commands, message);
    }

    fn delete_selected(self: &Rc<Self>) {
        let selected = self.selected();
        if selected.is_empty() {
            return;
        }
        let trash = self.capabilities().trash;
        let commands: Vec<Command> = selected
            .iter()
            .filter_map(|e| {
                self.remote(&e.path).map(|path| Command::Delete {
                    path,
                    recursive: true,
                })
            })
            .collect();
        let n = selected.len();
        if trash {
            self.run_all(
                commands,
                ngettext_n(
                    "{} item moved to the trash",
                    "{} items moved to the trash",
                    n,
                ),
            );
            return;
        }
        let this = Rc::clone(self);
        glib::spawn_future_local(async move {
            let heading = if n == 1 {
                gettext("Delete “{}” Permanently?").replace("{}", selected[0].display_name())
            } else {
                gettext("Delete {} Items Permanently?").replace("{}", &n.to_string())
            };
            let dialog = adw::AlertDialog::builder()
                .heading(heading)
                .body(gettext(
                    "This account has no trash: deleted items cannot be recovered.",
                ))
                .build();
            dialog.add_response("cancel", &gettext("_Cancel"));
            dialog.add_response("delete", &gettext("_Delete"));
            dialog.set_response_appearance("delete", adw::ResponseAppearance::Destructive);
            dialog.set_default_response(Some("cancel"));
            dialog.set_close_response("cancel");
            if dialog.choose_future(Some(&this.window)).await == "delete" {
                this.run_all(
                    commands,
                    ngettext_n("Deleted {} item", "Deleted {} items", n),
                );
            }
        });
    }

    fn copy_link(self: &Rc<Self>) {
        let Some(entry) = self.single() else {
            return;
        };
        let Some(path) = self.remote(&entry.path) else {
            return;
        };
        let this = Rc::clone(self);
        self.run(Command::PublicLink { path }, move |reply| {
            if let Reply::Link(url) = reply {
                this.window.clipboard().set_text(&url);
                this.toast(&gettext("Link copied"));
            }
        });
    }

    fn properties(&self) {
        let Some(entry) = self.single() else {
            return;
        };
        let Some(status) = self.account() else { return };
        let group = adw::PreferencesGroup::new();
        let add = |title: String, value: String| {
            if value.is_empty() {
                return;
            }
            let row = adw::ActionRow::builder()
                .title(title)
                .subtitle(glib::markup_escape_text(&value))
                .subtitle_selectable(true)
                .css_classes(["property"])
                .build();
            group.add(&row);
        };
        let kind = if entry.is_dir() {
            gettext("Folder")
        } else if entry.native_doc.is_some() {
            gettext("Web document")
        } else {
            entry
                .mime
                .as_deref()
                .map(|m| gio::content_type_get_description(m).to_string())
                .unwrap_or_default()
        };
        add(gettext("Type"), kind);
        add(
            gettext("Size"),
            entry
                .size
                .map(|s| format!("{} ({s} bytes)", human_size(s)))
                .unwrap_or_default(),
        );
        add(gettext("Modified"), modified_text(&entry));
        add(
            gettext("Location"),
            format!("{}:{}", status.account.display_name, parent(&entry.path)),
        );
        if status.mounted && !entry.has(Badge::NotSynced) {
            add(
                gettext("In the folder"),
                status
                    .account
                    .mount_point
                    .join(&entry.path)
                    .display()
                    .to_string(),
            );
        }
        add(gettext("State"), state_label(&entry));
        add(gettext("Checksum"), entry.hash.clone().unwrap_or_default());
        let page = adw::PreferencesPage::new();
        page.add(&group);
        let view = adw::ToolbarView::new();
        view.add_top_bar(&adw::HeaderBar::new());
        view.set_content(Some(&page));
        let dialog = adw::Dialog::builder()
            .title(entry.display_name())
            .content_width(420)
            .child(&view)
            .build();
        dialog.present(Some(&self.window));
    }

    fn show_in_files(&self) {
        let Some(entry) = self.single() else {
            return;
        };
        let Some(status) = self.account().filter(|a| a.mounted) else {
            return;
        };
        let file = gio::File::for_path(status.account.mount_point.join(&entry.path));
        let toasts = self.toasts.clone();
        gtk::FileLauncher::new(Some(&file)).open_containing_folder(
            Some(&self.window),
            gio::Cancellable::NONE,
            move |result| {
                if let Err(e) = result
                    && !e.matches(gtk::DialogError::Dismissed)
                {
                    toasts.add_toast(adw::Toast::new(&e.to_string()));
                }
            },
        );
    }

    fn show_folder(&self) {
        let Some(status) = self.account().filter(|a| a.mounted) else {
            return;
        };
        let path = status.account.mount_point.join(&self.state.borrow().path);
        open_folder(&self.window, &path);
    }

    /// "Exclude from sync" / "Include in sync". Applying the rule remounts
    /// the account: with uploads pending, the daemon asks for a
    /// confirmation first.
    fn set_excluded(self: &Rc<Self>, exclude: bool) {
        let Some(entry) = self.single() else {
            return;
        };
        let Some(path) = self.remote(&entry.path) else {
            return;
        };
        let this = Rc::clone(self);
        glib::spawn_future_local(async move {
            let command = |force| {
                let rule = ExclusionRule::Path(path.clone());
                let account = path.account.clone();
                if exclude {
                    Command::ExcludeAdd {
                        account,
                        rule,
                        force,
                    }
                } else {
                    Command::ExcludeRemove {
                        account,
                        rule,
                        force,
                    }
                }
            };
            this.set_busy(1);
            let mut answer = this.ui.backend.request(command(false)).await;
            this.set_busy(-1);
            if let Ok(Err(Error::Busy(message))) = &answer {
                let dialog = adw::AlertDialog::builder()
                    .heading(gettext("Files Are Waiting to Upload"))
                    .body(message)
                    .build();
                dialog.add_response("cancel", &gettext("_Cancel"));
                dialog.add_response("apply", &gettext("_Apply Now"));
                dialog.set_default_response(Some("cancel"));
                dialog.set_close_response("cancel");
                if dialog.choose_future(Some(&this.window)).await != "apply" {
                    return;
                }
                this.set_busy(1);
                answer = this.ui.backend.request(command(true)).await;
                this.set_busy(-1);
            }
            match answer {
                Ok(Ok(_)) => {
                    this.toast(
                        &if exclude {
                            gettext("“{}” is no longer synced to this computer")
                        } else {
                            gettext("“{}” is synced again")
                        }
                        .replace("{}", entry.display_name()),
                    );
                    this.load();
                }
                Ok(Err(Error::NotFound(_))) if !exclude => this.toast(&gettext(
                    "This item is excluded by a pattern. Change the exclusions in the preferences.",
                )),
                Ok(Err(e)) => this.toast(&e.to_string()),
                Err(message) => this.toast(&message),
            }
        });
    }

    /// Shows the window, at `account` if given.
    pub fn present(self: &Rc<Self>, account: Option<&AccountId>) {
        if let Some(id) = account {
            let current = self.state.borrow().account.clone();
            if current.as_ref() != Some(id) {
                self.navigate(id, "");
            } else {
                self.load();
            }
            self.split.set_show_content(true);
        }
        self.window.present();
    }
}

/// `{}` replaced by `n`, singular or plural.
fn ngettext_n(singular: &str, plural: &str, n: usize) -> String {
    gettextrs::ngettext(singular, plural, n as u32).replace("{}", &n.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names() {
        assert_eq!(copy_name("report.odt"), "report (copy).odt");
        assert_eq!(copy_name("notes"), "notes (copy)");
        assert_eq!(copy_name(".bashrc"), ".bashrc (copy)");
        assert_eq!(parent("a/b"), "a");
        assert_eq!(join("", "a"), "a");
    }
}
