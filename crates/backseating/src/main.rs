#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]
use diffreview::DiffRequest;
use diffreview_ui::{LocalBackend, ProjectStore, ReviewEvent, ReviewOptions, ReviewView};
use gpui_kit::component::button::Button;
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::{ActiveTheme, Sizable, TitleBar, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{
    App, AppContext, Bounds, Render, Window, WindowBounds, WindowOptions, div, px, size,
};
use std::rc::Rc;
struct Hub {
    store: ProjectStore,
    review: Option<gpui_kit::Entity<ReviewView>>,
    path_input: gpui_kit::Entity<InputState>,
    font_input: gpui_kit::Entity<InputState>,
}
impl Render for Hub {
    fn render(
        &mut self,
        _window: &mut Window,
        cx: &mut gpui_kit::Context<Self>,
    ) -> impl gpui_kit::IntoElement {
        let theme = cx.theme().clone();
        if let Some(review) = &self.review {
            return v_flex().size_full().child(review.clone());
        }
        let projects = self.store.projects.iter().map(|project| {
            let path = project.path.clone();
            h_flex()
                .w_full()
                .justify_between()
                .p_2()
                .border_b_1()
                .border_color(theme.border)
                .child(
                    div().text_sm().child(
                        project
                            .name
                            .clone()
                            .unwrap_or_else(|| project.path.display().to_string()),
                    ),
                )
                .child(
                    Button::new(project.path.to_string_lossy().to_string())
                        .small()
                        .label("Open")
                        .on_click(cx.listener(move |this, _, window, cx| {
                            let backend = Rc::new(LocalBackend::new(path.clone()));
                            let review = cx.new(|cx| {
                                ReviewView::new(
                                    backend,
                                    DiffRequest::default(),
                                    ReviewOptions {
                                        discard_label: Some("Projects".into()),
                                        ..ReviewOptions::default()
                                    },
                                    window,
                                    cx,
                                )
                            });
                            cx.subscribe(&review, |this, _, event: &ReviewEvent, cx| {
                                if matches!(event, ReviewEvent::Discard) {
                                    this.review = None;
                                    cx.notify();
                                }
                            })
                            .detach();
                            this.review = Some(review);
                            cx.notify();
                        })),
                )
        });
        v_flex()
            .size_full()
            .gap_3()
            .p_6()
            .bg(theme.background)
            .child(div().text_2xl().child("Backseating"))
            .child(
                div()
                    .text_sm()
                    .text_color(theme.muted_foreground)
                    .child("Choose a project to review"),
            )
            .child(
                h_flex()
                    .gap_2()
                    .child(Input::new(&self.path_input).small())
                    .child(
                        Button::new("add-project")
                            .small()
                            .label("Add project")
                            .on_click(cx.listener(|this, _, _, cx| {
                                let path = this.path_input.read(cx).value().trim().to_owned();
                                if !path.is_empty() {
                                    this.store.add(path);
                                    let _ = this.store.save();
                                    cx.notify();
                                }
                            })),
                    ),
            )
            .child(
                h_flex()
                    .gap_2()
                    .child(div().text_sm().child("Code font"))
                    .child(Input::new(&self.font_input).small())
                    .child(
                        Button::new("save-font")
                            .small()
                            .label("Save")
                            .on_click(cx.listener(|this, _, _, cx| {
                                let value = this.font_input.read(cx).value().trim().to_owned();
                                this.store.code_font = (!value.is_empty()).then_some(value);
                                let _ = this.store.save();
                                cx.notify();
                            })),
                    ),
            )
            .child(v_flex().gap_1().children(projects))
    }
}
fn start(cx: &mut App) {
    gpui_kit::init(cx);
    diffreview_ui::init(cx);
    let mut store = ProjectStore::load().unwrap_or_default();
    if let Some(path) = std::env::args_os().nth(1) {
        store.add(path);
        let _ = store.save();
    }
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
            None,
            size(px(900.), px(650.)),
            cx,
        ))),
        ..TitleBar::window_options()
    };
    let _ = gpui_kit::open_window(options, cx, move |window, cx| {
        let path_input = cx.new(|cx| InputState::new(window, cx).placeholder("/path/to/project"));
        let font_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("monospace font (optional)"));
        cx.new(|_| Hub {
            store,
            review: None,
            path_input,
            font_input,
        })
    });
}
fn main() {
    gpui_kit::application()
        .with_assets(gpui_kit::assets::Assets)
        .run(start);
}
