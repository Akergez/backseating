//! The settings page: how the window looks — light or dark, the colours, the
//! themes and the fonts. Everything takes effect as it is chosen, since the
//! only way to judge a theme or a font is to see it. The theme lists and the
//! registry browser are the engine's own (`gpui-zed-themes`); this page only
//! lays them out, as Aiddiction's does.

use gpui_adaptive_colors::Color;
use gpui_kit::component::button::{Button, ButtonGroup};
use gpui_kit::component::select::{SearchableVec, Select, SelectEvent, SelectState};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{
    ActiveTheme, IndexPath, Selectable as _, Sizable as _, StyledExt as _, ThemeMode, h_flex,
    v_flex,
};
use gpui_kit::prelude::*;
use gpui_kit::{App, AppContext, Context, Entity, Render, SharedString, Subscription, Window, div};
use gpui_zed_themes::{Browser, BrowserEvent};

use crate::appearance::{self, FontRole, Mode, SEEDS};

/// A searchable list of names: themes, or font families.
type Names = SelectState<SearchableVec<SharedString>>;

pub struct SettingsPage {
    light: Entity<Names>,
    dark: Entity<Names>,
    interface_font: Entity<Names>,
    code_font: Entity<Names>,
    browser: Entity<Browser>,
    browsing: bool,
    _subscriptions: Vec<Subscription>,
}

impl SettingsPage {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut subscriptions = Vec::new();

        let mut theme_picker = |mode: ThemeMode, cx: &mut Context<Self>| {
            let names = appearance::theme_names(mode, cx);
            let worn = appearance::worn(mode, cx);
            let selected = names.iter().position(|name| *name == worn).unwrap_or(0);
            let state = cx.new(|cx| {
                SelectState::new(
                    SearchableVec::new(names),
                    Some(IndexPath::default().row(selected)),
                    window,
                    cx,
                )
                .searchable(true)
            });
            subscriptions.push(cx.subscribe(
                &state,
                move |_, _, event: &SelectEvent<SearchableVec<SharedString>>, cx| {
                    if let SelectEvent::Confirm(Some(name)) = event {
                        appearance::choose_theme(mode, name, cx);
                    }
                },
            ));
            state
        };
        let light = theme_picker(ThemeMode::Light, cx);
        let dark = theme_picker(ThemeMode::Dark, cx);

        let (interface_default, code_default) = appearance::built_in_fonts(cx);
        let mut families: Vec<SharedString> = cx
            .text_system()
            .all_font_names()
            .into_iter()
            .map(SharedString::from)
            .collect();
        families.sort_by_key(|family| family.to_lowercase());
        families.dedup();
        let mut font_picker = |role: FontRole, default: &SharedString, cx: &mut Context<Self>| {
            // The first entry is the built-in font, by name, so "the
            // default" is not a mystery.
            let built_in: SharedString = format!("{default} (default)").into();
            let mut items = vec![built_in.clone()];
            items.extend(families.iter().cloned());
            // A chosen font that is no longer installed reads as the
            // built-in one, which is also what is drawn.
            let selected = appearance::font_choice(role, cx)
                .and_then(|chosen| families.iter().position(|family| *family == chosen))
                .map_or(0, |position| position + 1);
            let state = cx.new(|cx| {
                SelectState::new(
                    SearchableVec::new(items),
                    Some(IndexPath::default().row(selected)),
                    window,
                    cx,
                )
                .searchable(true)
            });
            subscriptions.push(cx.subscribe(
                &state,
                move |_, _, event: &SelectEvent<SearchableVec<SharedString>>, cx| {
                    let SelectEvent::Confirm(Some(family)) = event else {
                        return;
                    };
                    let family = (*family != built_in).then(|| family.to_string());
                    appearance::set_font_choice(role, family, cx);
                },
            ));
            state
        };
        let interface_font = font_picker(FontRole::Interface, &interface_default, cx);
        let code_font = font_picker(FontRole::Code, &code_default, cx);

        let browser =
            cx.new(|cx| Browser::new(appearance::registry(), appearance::store(), window, cx));
        subscriptions.push(cx.subscribe_in(
            &browser,
            window,
            |this, _, _: &BrowserEvent, window, cx| {
                appearance::reload_themes(cx);
                this.relist(window, cx);
            },
        ));
        Self {
            light,
            dark,
            interface_font,
            code_font,
            browser,
            browsing: false,
            _subscriptions: subscriptions,
        }
    }

    /// Makes the two theme lists say what is installed now.
    fn relist(&self, window: &mut Window, cx: &mut Context<Self>) {
        for (state, mode) in [
            (&self.light, ThemeMode::Light),
            (&self.dark, ThemeMode::Dark),
        ] {
            let names = appearance::theme_names(mode, cx);
            let worn = appearance::worn(mode, cx);
            state.update(cx, |state, cx| {
                state.set_items(SearchableVec::new(names), window, cx);
                state.set_selected_value(&worn, window, cx);
            });
        }
        cx.notify();
    }

    fn browse(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.browsing = !self.browsing;
        if self.browsing {
            // The first opening asks for the list, and so does one after a
            // failure.
            self.browser.update(cx, |browser, cx| {
                browser.load(cx);
                browser.focus(window, cx);
            });
        }
        cx.notify();
    }
}

fn row(title: &'static str, subtitle: &'static str, cx: &App) -> impl IntoElement {
    v_flex().flex_1().min_w_0().child(title).child(
        div()
            .text_xs()
            .text_color(cx.theme().muted_foreground)
            .child(subtitle),
    )
}

fn colors(current: Option<Color>, cx: &App) -> impl IntoElement {
    let (ring, gap) = (cx.theme().foreground, cx.theme().background);
    h_flex()
        .flex_wrap()
        .gap_2()
        .child(
            Button::new("color-system")
                .label("System")
                .outline()
                .small()
                .selected(current.is_none())
                .on_click(|_, _, cx| appearance::set_color(None, cx)),
        )
        .children(SEEDS.into_iter().filter_map(|(stored, name)| {
            let color = Color::parse_hex(stored)?;
            Some(
                div()
                    .id(stored)
                    .flex_none()
                    .size_6()
                    .rounded_full()
                    .cursor_pointer()
                    .when(current == Some(color), |swatch| {
                        swatch.border_2().border_color(ring)
                    })
                    .child(
                        div()
                            .size_full()
                            .rounded_full()
                            .border_2()
                            .border_color(gap)
                            .bg(gpui_adaptive_colors::hsla(color)),
                    )
                    .tooltip(move |window, cx| Tooltip::new(name).build(window, cx))
                    .on_click(move |_, _, cx| appearance::set_color(Some(color), cx)),
            )
        }))
}

impl Render for SettingsPage {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mode = appearance::mode(cx);
        let select = |state: &Entity<Names>, placeholder: &'static str| {
            div()
                .flex_none()
                .w_64()
                .child(Select::new(state).small().search_placeholder(placeholder))
        };
        let section = |title: &'static str| div().pt_2().text_lg().font_semibold().child(title);
        v_flex()
            .id("settings")
            .size_full()
            .overflow_y_scroll()
            .bg(cx.theme().background)
            .child(
                v_flex()
                    .w_full()
                    .max_w(gpui_kit::rems(44.))
                    .mx_auto()
                    .p_6()
                    .gap_4()
                    .child(section("Appearance"))
                    .child(
                        h_flex()
                            .gap_3()
                            .child(row("Mode", "System follows the desktop as it changes", cx))
                            .child(
                                ButtonGroup::new("mode")
                                    .outline()
                                    .small()
                                    .children(Mode::ALL.map(|choice| {
                                        Button::new(choice.label())
                                            .label(choice.label())
                                            .selected(choice == mode)
                                    }))
                                    .on_click(|picked, window, cx| {
                                        if let Some(choice) =
                                            picked.first().and_then(|index| Mode::ALL.get(*index))
                                        {
                                            appearance::set_mode(*choice, window, cx);
                                        }
                                    }),
                            ),
                    )
                    .child(
                        v_flex()
                            .gap_2()
                            .child(row(
                                "Color",
                                "The scheme is made from it; System follows the desktop's accent",
                                cx,
                            ))
                            .child(colors(appearance::color(cx), cx)),
                    )
                    .child(
                        h_flex()
                            .gap_3()
                            .child(row(
                                "Light theme",
                                "Worn in light, instead of the scheme",
                                cx,
                            ))
                            .child(select(&self.light, "Search themes")),
                    )
                    .child(
                        h_flex()
                            .gap_3()
                            .child(row("Dark theme", "Worn in dark, instead of the scheme", cx))
                            .child(select(&self.dark, "Search themes")),
                    )
                    .child(
                        h_flex()
                            .gap_3()
                            .child(row("More themes", "From the Zed editor's registry", cx))
                            .child(
                                Button::new("browse-themes")
                                    .small()
                                    .outline()
                                    .label(if self.browsing { "Done" } else { "Browse…" })
                                    .on_click(
                                        cx.listener(|this, _, window, cx| this.browse(window, cx)),
                                    ),
                            ),
                    )
                    .when(self.browsing, |column| column.child(self.browser.clone()))
                    .child(section("Fonts"))
                    .child(
                        h_flex()
                            .gap_3()
                            .child(row(
                                "Interface font",
                                "Buttons, lists and everything else",
                                cx,
                            ))
                            .child(select(&self.interface_font, "Search fonts")),
                    )
                    .child(
                        h_flex()
                            .gap_3()
                            .child(row("Code font", "The diff and the file names", cx))
                            .child(select(&self.code_font, "Search fonts")),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(
                                "A variable font is loaded as its regular face only, so bold \
                                 text will not look bold in one.",
                            ),
                    ),
            )
    }
}
