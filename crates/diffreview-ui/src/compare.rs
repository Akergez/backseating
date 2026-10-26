//! The dialog that picks what to compare: a base and a target, each a point
//! in the repository's history — HEAD, a branch, a remote's branch, a tag, a
//! recent commit, or any ref typed in — and the target also the working tree
//! or the index. Searchable lists over [`Refs`]; the choice is a
//! [`Comparison`], which is a [`DiffSource`].

use diffreview::{Comparison, DiffSource, Point, Refs};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::select::{SearchableGroup, SearchableListItem, SearchableVec, Select, SelectState};
use gpui_kit::component::{ActiveTheme, IndexPath, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{App, AppContext, Context, Entity, Render, SharedString, Window, div};

/// One row of a list: what it says, what it says beside that, and the point.
#[derive(Debug, Clone)]
pub struct RefItem {
    title: SharedString,
    detail: Option<SharedString>,
    point: Point,
}

impl RefItem {
    fn named(name: &str, detail: Option<&str>) -> Self {
        Self {
            title: name.to_owned().into(),
            detail: detail.map(|detail| detail.to_owned().into()),
            point: Point::Ref { name: name.to_owned() },
        }
    }
}

impl SearchableListItem for RefItem {
    type Value = Point;

    fn title(&self) -> SharedString {
        self.title.clone()
    }

    fn render(&self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = cx.theme();
        h_flex()
            .w_full()
            .min_w_0()
            .gap_2()
            .child(div().flex_none().font_family(theme.mono_font_family.clone()).child(self.title.clone()))
            .children(self.detail.clone().map(|detail| {
                div().flex_1().min_w_0().truncate().text_color(theme.muted_foreground).child(detail)
            }))
    }

    fn value(&self) -> &Point {
        &self.point
    }

    /// A commit is found by its subject as well as its hash.
    fn matches(&self, query: &str) -> bool {
        let query = query.to_lowercase();
        self.title.to_lowercase().contains(&query)
            || self.detail.as_ref().is_some_and(|detail| detail.to_lowercase().contains(&query))
    }
}

type Groups = SearchableVec<SearchableGroup<RefItem>>;

/// The lists, grouped. The target's begins with the working tree and the
/// index; `extra` is a point the lists may not have (the one in use, typed
/// in before), put first so it can be chosen again.
fn groups(refs: &Refs, target: bool, extra: Option<&Point>) -> Vec<SearchableGroup<RefItem>> {
    let mut groups = Vec::new();
    let mut here = SearchableGroup::new("Here");
    if target {
        here = here
            .item(RefItem { title: "Working tree".into(), detail: Some("the files on disk".into()), point: Point::WorkingTree })
            .item(RefItem { title: "Staged".into(), detail: Some("the index, against HEAD".into()), point: Point::Staged });
    }
    here = here.item(RefItem::named("HEAD", refs.current_branch.as_deref().or(Some("detached"))));
    groups.push(here);
    let known = |name: &str| {
        name == "HEAD"
            || refs.branches.iter().chain(&refs.remote_branches).chain(&refs.tags).any(|known| known == name)
            || refs.commits.iter().any(|(hash, _)| hash == name)
    };
    if let Some(Point::Ref { name }) = extra
        && !known(name) {
            groups.insert(0, SearchableGroup::new("In use").item(RefItem::named(name, None)));
        }
    let named = |title: &str, names: &[String], detail: &dyn Fn(&str) -> Option<&'static str>| {
        SearchableGroup::new(title.to_owned()).items(names.iter().map(|name| RefItem::named(name, detail(name))))
    };
    let current = refs.current_branch.clone();
    let default = refs.default_branch.clone();
    let branch_detail = move |name: &str| {
        if current.as_deref() == Some(name) {
            Some("current")
        } else if default.as_deref() == Some(name) {
            Some("default")
        } else {
            None
        }
    };
    for (title, names, detail) in [
        ("Branches", &refs.branches, &branch_detail as &dyn Fn(&str) -> Option<&'static str>),
        ("Remote branches", &refs.remote_branches, &|_: &str| None),
        ("Tags", &refs.tags, &|_: &str| None),
    ] {
        if !names.is_empty() {
            groups.push(named(title, names, detail));
        }
    }
    if !refs.commits.is_empty() {
        groups.push(
            SearchableGroup::new("Commits")
                .items(refs.commits.iter().map(|(hash, subject)| RefItem::named(hash, Some(subject)))),
        );
    }
    groups
}

/// Where `point` is in `groups`.
fn position(groups: &[SearchableGroup<RefItem>], point: &Point) -> Option<IndexPath> {
    groups.iter().enumerate().find_map(|(section, group)| {
        group.items.iter().position(|item| item.point == *point).map(|row| IndexPath::default().section(section).row(row))
    })
}

pub struct ComparePicker {
    base: Entity<SelectState<Groups>>,
    target: Entity<SelectState<Groups>>,
    base_typed: Entity<InputState>,
    target_typed: Entity<InputState>,
    since: bool,
    error: Option<SharedString>,
}

impl ComparePicker {
    pub fn new(refs: &Refs, current: &DiffSource, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let comparison = Comparison::from(current);
        let base_point = Point::Ref { name: comparison.base.clone() };
        let select = |target: bool, point: &Point, window: &mut Window, cx: &mut Context<Self>| {
            let groups = groups(refs, target, Some(point));
            let selected = position(&groups, point);
            cx.new(|cx| SelectState::new(SearchableVec::new(groups), selected, window, cx).searchable(true))
        };
        let base = select(false, &base_point, window, cx);
        let target = select(true, &comparison.target, window, cx);
        // A change of target can turn the base off and on.
        cx.observe(&target, |_, _, cx| cx.notify()).detach();
        let typed = |placeholder: &'static str, window: &mut Window, cx: &mut Context<Self>| {
            cx.new(|cx| InputState::new(window, cx).placeholder(placeholder))
        };
        Self {
            base,
            target,
            base_typed: typed("or any ref: HEAD~3, a hash…", window, cx),
            target_typed: typed("or any ref", window, cx),
            since: comparison.since,
            error: None,
        }
    }

    fn target_point(&self, cx: &App) -> Option<Point> {
        let typed = self.target_typed.read(cx).value().trim().to_owned();
        if !typed.is_empty() {
            return Some(Point::Ref { name: typed });
        }
        self.target.read(cx).selected_value().cloned()
    }

    fn base_name(&self, cx: &App) -> Option<String> {
        let typed = self.base_typed.read(cx).value().trim().to_owned();
        if !typed.is_empty() {
            return Some(typed);
        }
        match self.base.read(cx).selected_value()? {
            Point::Ref { name } => Some(name.clone()),
            _ => None,
        }
    }

    /// What is picked, or why nothing is. The error is kept to be shown.
    pub fn source(&mut self, cx: &mut Context<Self>) -> Result<DiffSource, String> {
        let result = (|| {
            let target = self.target_point(cx).ok_or("Pick what to compare.")?;
            let base = match target {
                Point::Staged => "HEAD".to_owned(),
                _ => self.base_name(cx).ok_or("Pick what to compare it with.")?,
            };
            let source = Comparison { base, target, since: self.since }.source();
            source.validate()?;
            Ok(source)
        })();
        self.error = result.as_ref().err().map(|error: &String| error.clone().into());
        cx.notify();
        result
    }
}

impl Render for ComparePicker {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let target = self.target_point(cx);
        let staged = target == Some(Point::Staged);
        let target_is_ref = matches!(target, Some(Point::Ref { .. }));
        let label = |text: &'static str| div().text_sm().font_weight(gpui_kit::FontWeight::MEDIUM).child(text);
        let select = |state: &Entity<SelectState<Groups>>, disabled: bool| {
            Select::new(state).small().menu_max_h(gpui_kit::rems(22.)).search_placeholder("Search").disabled(disabled)
        };
        let preview: SharedString = match (self.base_name(cx), &target) {
            (_, Some(Point::Staged)) => "git diff --cached".into(),
            (Some(base), Some(Point::WorkingTree)) => format!("git diff {base}").into(),
            (Some(base), Some(Point::Ref { name })) if self.since => format!("git diff {base}...{name}").into(),
            (Some(base), Some(Point::Ref { name })) => format!("git diff {base} {name}").into(),
            _ => SharedString::default(),
        };
        v_flex()
            .gap_3()
            .child(
                v_flex()
                    .gap_1()
                    .child(label("Changes in"))
                    .child(select(&self.target, false))
                    .child(Input::new(&self.target_typed).small()),
            )
            .child(
                v_flex()
                    .gap_1()
                    .child(label("Compared with"))
                    .child(select(&self.base, staged))
                    .child(Input::new(&self.base_typed).small().disabled(staged)),
            )
            .child(
                Checkbox::new("since")
                    .label("Only what it added since it branched off (three dots)")
                    .checked(self.since && target_is_ref)
                    .disabled(!target_is_ref)
                    .on_click(cx.listener(|this, checked: &bool, _, cx| {
                        this.since = *checked;
                        cx.notify();
                    })),
            )
            .child(
                div()
                    .text_xs()
                    .font_family(theme.mono_font_family.clone())
                    .text_color(theme.muted_foreground)
                    .child(preview),
            )
            .when_some(self.error.clone(), |column, error| column.child(div().text_sm().text_color(theme.danger).child(error)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn refs() -> Refs {
        Refs {
            current_branch: Some("feature".into()),
            default_branch: Some("main".into()),
            branches: vec!["feature".into(), "main".into()],
            commits: vec![("abc1234".into(), "Fix the parser".into())],
            remote_branches: vec!["origin/main".into()],
            tags: vec![],
        }
    }

    #[test]
    fn the_target_offers_the_working_tree_and_empty_groups_are_left_out() {
        let titles = |groups: &[SearchableGroup<RefItem>]| groups.iter().map(|group| group.title.to_string()).collect::<Vec<_>>();
        let target = groups(&refs(), true, None);
        assert_eq!(titles(&target), ["Here", "Branches", "Remote branches", "Commits"]);
        assert_eq!(target[0].items[0].point, Point::WorkingTree);
        let base = groups(&refs(), false, None);
        assert_eq!(base[0].items.len(), 1);
        assert_eq!(position(&base, &Point::Ref { name: "main".into() }), Some(IndexPath::default().section(1).row(1)));
    }

    #[test]
    fn a_point_the_lists_lack_is_offered_first() {
        let extra = Point::Ref { name: "HEAD~3".into() };
        let base = groups(&refs(), false, Some(&extra));
        assert_eq!(base[0].title.as_ref(), "In use");
        assert_eq!(position(&base, &extra), Some(IndexPath::default()));
        let known = Point::Ref { name: "abc1234".into() };
        assert_eq!(groups(&refs(), false, Some(&known)).len(), base.len() - 1);
    }

    #[test]
    fn commits_are_found_by_subject() {
        let item = RefItem::named("abc1234", Some("Fix the parser"));
        assert!(item.matches("parser"));
        assert!(item.matches("ABC"));
        assert!(!item.matches("lexer"));
    }
}
