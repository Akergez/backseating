#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]
//! Backseating: the review of a git working tree, one tab for each project.
//!
//! The projects are the tabs; a folder is added with the system's folder
//! picker (or as the first argument) and stays until its tab is closed. The
//! settings — light or dark, colours, themes, fonts — are one more tab.

mod appearance;
mod settings;
mod settings_page;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use diffreview::DiffRequest;
use diffreview_ui::{LocalBackend, ProjectStore, ReviewEvent, ReviewOptions, ReviewView};
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::tab::{Tab, TabBar};
use gpui_kit::component::{
    ActiveTheme, IconName, Sizable as _, TitleBar, WindowExt as _, h_flex, v_flex,
};
use gpui_kit::prelude::*;
use gpui_kit::{
    App, AppContext, Bounds, ClipboardItem, Context, Entity, KeyBinding, PathPromptOptions, Render,
    Subscription, TitlebarOptions, Window, WindowBounds, WindowOptions, div, px, size,
};

use settings::Settings;
use settings_page::SettingsPage;

gpui_kit::actions!(backseating, [OpenProject, OpenSettings, CloseTab]);

/// What the window shows under the tabs.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Page {
    Project(PathBuf),
    Settings,
    /// No project yet.
    Welcome,
}

struct Hub {
    store: ProjectStore,
    /// Made when a project's tab is first shown, and kept while it is open:
    /// switching tabs keeps a review's annotations.
    reviews: HashMap<PathBuf, (Entity<ReviewView>, Subscription)>,
    /// Made when the settings are first opened.
    settings: Option<Entity<SettingsPage>>,
    /// Whether the settings have a tab, and whether it is the one shown.
    settings_open: bool,
    showing_settings: bool,
    _appearance: Vec<Subscription>,
}

impl Hub {
    fn new(store: ProjectStore, window: &mut Window, cx: &mut Context<Self>) -> Self {
        // Only now is there a window to ask what the desktop looks like.
        appearance::apply_mode(Some(window), cx);
        let subscriptions = vec![
            cx.observe_window_appearance(window, |_, window, cx| {
                appearance::apply_mode(Some(window), cx)
            }),
            // A desktop's accent colour can change while the window is in
            // the background; it is asked again on the way back.
            cx.observe_window_activation(window, |_, window, cx| {
                if window.is_window_active() {
                    gpui_adaptive_colors::refresh(cx);
                }
            }),
        ];
        Self {
            store,
            reviews: HashMap::new(),
            settings: None,
            settings_open: false,
            showing_settings: false,
            _appearance: subscriptions,
        }
    }

    fn page(&self) -> Page {
        if self.showing_settings {
            return Page::Settings;
        }
        self.store
            .last_project
            .clone()
            .filter(|path| {
                self.store
                    .projects
                    .iter()
                    .any(|project| project.path == *path)
            })
            .or_else(|| {
                self.store
                    .projects
                    .first()
                    .map(|project| project.path.clone())
            })
            .map_or(Page::Welcome, Page::Project)
    }

    fn save(&self) {
        if let Err(error) = self.store.save() {
            eprintln!("backseating: {error}");
        }
    }

    /// Asks the system for folders and opens each as a project.
    fn pick_folders(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let picked = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: true,
            prompt: Some("Open".into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = match picked.await {
                Ok(result) => result,
                // The picker went away without an answer.
                Err(_) => return,
            };
            let _ = this.update_in(cx, |this, window, cx| match result {
                Ok(Some(paths)) => this.open_projects(paths, window, cx),
                // Cancelled.
                Ok(None) => {}
                // No portal, or no picker behind it: say so rather than do
                // nothing.
                Err(error) => window
                    .push_notification(format!("Could not open the folder picker: {error}"), cx),
            });
        })
        .detach();
    }

    fn open_projects(&mut self, paths: Vec<PathBuf>, window: &mut Window, cx: &mut Context<Self>) {
        for path in paths {
            self.store.add(path);
        }
        self.save();
        self.showing_settings = false;
        self.focus_page(window, cx);
        cx.notify();
    }

    fn show_project(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        self.store.last_project = Some(path);
        self.showing_settings = false;
        self.save();
        self.focus_page(window, cx);
        cx.notify();
    }

    fn show_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.settings.is_none() {
            self.settings = Some(cx.new(|cx| SettingsPage::new(window, cx)));
        }
        self.settings_open = true;
        self.showing_settings = true;
        cx.notify();
    }

    /// Closes the tab shown: the settings, or a project, which is then
    /// forgotten. The tab beside it is shown in its place.
    fn close_current(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.page() {
            Page::Settings => self.close_settings(window, cx),
            Page::Project(path) => self.close_project(&path, window, cx),
            Page::Welcome => {}
        }
    }

    fn close_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.settings_open = false;
        self.showing_settings = false;
        self.focus_page(window, cx);
        cx.notify();
    }

    fn close_project(&mut self, path: &Path, window: &mut Window, cx: &mut Context<Self>) {
        let Some(index) = self
            .store
            .projects
            .iter()
            .position(|project| project.path == path)
        else {
            return;
        };
        let was_shown = self.page() == Page::Project(path.to_owned());
        self.store.remove(path);
        self.reviews.remove(path);
        if was_shown || self.store.last_project.is_none() {
            let projects = &self.store.projects;
            self.store.last_project = projects
                .get(index)
                .or_else(|| projects.last())
                .map(|project| project.path.clone());
        }
        self.save();
        self.focus_page(window, cx);
        cx.notify();
    }

    /// The review of a project, made the first time it is asked for.
    fn review(
        &mut self,
        path: &Path,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<ReviewView> {
        if let Some((review, _)) = self.reviews.get(path) {
            return review.clone();
        }
        let backend = Rc::new(LocalBackend::new(path.to_owned()));
        let review = cx.new(|cx| {
            ReviewView::new(
                backend,
                DiffRequest::default(),
                ReviewOptions {
                    submit_label: "Copy review".into(),
                    discard_label: None,
                },
                window,
                cx,
            )
        });
        // There is no agent here to send the review to: it goes to the
        // clipboard, and to standard output for a script that started us.
        let subscription =
            cx.subscribe_in(&review, window, |_, _, event: &ReviewEvent, window, cx| {
                if let ReviewEvent::Submit(text) = event {
                    print!("{text}");
                    cx.write_to_clipboard(ClipboardItem::new_string(text.clone()));
                    let message = if text.is_empty() {
                        "No annotations to copy"
                    } else {
                        "Review copied to the clipboard"
                    };
                    window.push_notification(message, cx);
                }
            });
        self.reviews
            .insert(path.to_owned(), (review.clone(), subscription));
        review
    }

    /// Puts the keyboard in what is shown, so its keys work at once.
    fn focus_page(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Page::Project(path) = self.page() {
            let review = self.review(&path, window, cx);
            review.read(cx).focus_handle().focus(window, cx);
        }
    }

    fn render_tabs(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let page = self.page();
        let mut tabs: Vec<Tab> = Vec::new();
        let mut selected = None;
        for (index, project) in self.store.projects.iter().enumerate() {
            let path = project.path.clone();
            if page == Page::Project(path.clone()) {
                selected = Some(index);
            }
            let label = project.name.clone().unwrap_or_else(|| folder_name(&path));
            tabs.push(Tab::new().label(label).suffix(close_button(
                ("close-project", index),
                cx.listener(move |this, _, window, cx| {
                    cx.stop_propagation();
                    this.close_project(&path, window, cx);
                }),
            )));
        }
        if self.settings_open {
            if page == Page::Settings {
                selected = Some(tabs.len());
            }
            tabs.push(Tab::new().label("Settings").suffix(close_button(
                "close-settings",
                cx.listener(|this, _, window, cx| {
                    cx.stop_propagation();
                    this.close_settings(window, cx);
                }),
            )));
        }
        let projects = self.store.projects.len();
        let mut bar = TabBar::new("tabs")
            .children(tabs)
            .on_click(cx.listener(move |this, index: &usize, window, cx| {
                match this.store.projects.get(*index) {
                    Some(project) => this.show_project(project.path.clone(), window, cx),
                    None if *index == projects => this.show_settings(window, cx),
                    None => {}
                }
            }))
            .suffix(
                h_flex()
                    .h_full()
                    .gap_1()
                    .px_2()
                    .child(
                        Button::new("open-project")
                            .ghost()
                            .xsmall()
                            .icon(IconName::Plus)
                            .tooltip("Open a project (Ctrl+O)")
                            .on_click(
                                cx.listener(|this, _, window, cx| this.pick_folders(window, cx)),
                            ),
                    )
                    .child(
                        Button::new("open-settings")
                            .ghost()
                            .xsmall()
                            .icon(IconName::Settings)
                            .tooltip("Settings (Ctrl+,)")
                            .on_click(
                                cx.listener(|this, _, window, cx| this.show_settings(window, cx)),
                            ),
                    ),
            );
        if let Some(selected) = selected {
            bar = bar.selected_index(selected);
        }
        bar
    }

    fn render_welcome(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let theme = cx.theme();
        v_flex()
            .size_full()
            .items_center()
            .justify_center()
            .gap_3()
            .child(div().text_2xl().child("Backseating"))
            .child(
                div()
                    .text_sm()
                    .text_color(theme.muted_foreground)
                    .child("Open a git repository to review what changed in it"),
            )
            .child(
                Button::new("welcome-open")
                    .primary()
                    .icon(IconName::FolderOpen)
                    .label("Open a project…")
                    .on_click(cx.listener(|this, _, window, cx| this.pick_folders(window, cx))),
            )
    }
}

impl Render for Hub {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let content = match self.page() {
            Page::Welcome => self.render_welcome(cx).into_any_element(),
            Page::Settings => match &self.settings {
                Some(settings) => settings.clone().into_any_element(),
                None => div().into_any_element(),
            },
            Page::Project(path) => {
                let review = self.review(&path, window, cx);
                // A window that opens, or loses what had the keyboard, gives
                // it to the review, whose keys are most of the application.
                if window.focused(cx).is_none() {
                    review.read(cx).focus_handle().focus(window, cx);
                }
                review.into_any_element()
            }
        };
        v_flex()
            .size_full()
            .bg(theme.background)
            .text_color(theme.foreground)
            .font_family(theme.font_family.clone())
            .on_action(
                cx.listener(|this, _: &OpenProject, window, cx| this.pick_folders(window, cx)),
            )
            .on_action(
                cx.listener(|this, _: &OpenSettings, window, cx| this.show_settings(window, cx)),
            )
            .on_action(cx.listener(|this, _: &CloseTab, window, cx| this.close_current(window, cx)))
            .child(self.render_tabs(cx))
            .child(div().flex_1().min_h_0().child(content))
    }
}

fn close_button(
    id: impl Into<gpui_kit::ElementId>,
    on_click: impl Fn(&gpui_kit::ClickEvent, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    Button::new(id)
        .ghost()
        .xsmall()
        .icon(IconName::Close)
        .tooltip("Close")
        .on_click(on_click)
}

/// A project's tab says the folder's name; the whole path is in the folder
/// itself, and two folders of one name are rare enough.
fn folder_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

fn start(cx: &mut App) {
    gpui_kit::init(cx);
    diffreview_ui::init(cx);
    cx.bind_keys([
        KeyBinding::new("ctrl-o", OpenProject, None),
        KeyBinding::new("ctrl-,", OpenSettings, None),
        KeyBinding::new("ctrl-w", CloseTab, None),
    ]);

    let mut store = ProjectStore::load().unwrap_or_default();
    let mut settings = Settings::load();
    // The code font used to be kept with the projects.
    if let Some(font) = store.code_font.take() {
        settings.appearance.code_font.get_or_insert(font);
        let _ = store.save();
    }
    cx.set_global(settings);
    appearance::init(cx);

    if let Some(path) = std::env::args_os().nth(1) {
        let path = PathBuf::from(path);
        store.add(std::fs::canonicalize(&path).unwrap_or(path));
        let _ = store.save();
    }
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
            None,
            size(px(1100.), px(750.)),
            cx,
        ))),
        titlebar: Some(TitlebarOptions {
            title: Some("Backseating".into()),
            ..TitleBar::title_bar_options()
        }),
        app_id: Some("app.akergez.Backseating".into()),
        ..TitleBar::window_options()
    };
    let _ = gpui_kit::open_window(options, cx, move |window, cx| {
        cx.new(|cx| {
            let mut hub = Hub::new(store, window, cx);
            hub.focus_page(window, cx);
            hub
        })
    });
}

fn main() {
    gpui_kit::application()
        .with_assets(gpui_kit::assets::Assets)
        .run(start);
}
