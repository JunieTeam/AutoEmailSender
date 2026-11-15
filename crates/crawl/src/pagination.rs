use regex::Regex;
use scraper::{ElementRef, Selector};

use crate::{
    PageUrl, Pagination,
    page::{visible_text, visible_words},
};
use std::sync::LazyLock;

#[derive(Clone, Copy)]
enum Kind {
    Forward,
    Last,
    Backward,
    First,
    Number(u32),
    Jump,
    Other,
}

pub(crate) struct Control<'a> {
    pub element: ElementRef<'a>,
    pub link: Option<PageUrl>,
    kind: Kind,
    text: String,
}

impl<'a> Control<'a> {
    pub fn new(element: ElementRef<'a>, text: &str, link: Option<PageUrl>) -> Self {
        let control_text = text.trim().to_owned();
        let text = text.trim().to_lowercase();
        let label = text.trim_matches(['<', '>', '«', '»', '‹', '›', '→', '←', ' ']);
        // A tab shows a part of the page, as a carousel's numbered dots do, not a page of a list.
        let kind = if element.value().attr("role") == Some("tab") {
            Kind::Other
        } else if element.value().attr("rel").is_some_and(|rel| {
            rel.split_whitespace()
                .any(|r| r.eq_ignore_ascii_case("next"))
        }) || matches!(text.as_str(), ">" | "»" | "›" | "→")
            || matches!(
                label,
                "下一页" | "下页" | "下翻" | "后页" | "next" | "next page"
            )
        {
            Kind::Forward
        } else if matches!(label, "末页" | "尾页" | "last" | "last page") {
            Kind::Last
        } else if matches!(text.as_str(), "<" | "«" | "‹" | "←")
            || matches!(label, "上一页" | "上页" | "previous" | "previous page")
        {
            Kind::Backward
        } else if matches!(label, "首页" | "第一页" | "first" | "first page") {
            Kind::First
        } else if label == "跳转" {
            Kind::Jump
        } else if let Ok(number) = text.parse() {
            Kind::Number(number)
        } else {
            Kind::Other
        };
        Self {
            element,
            link,
            kind,
            text: control_text,
        }
    }
}

pub(crate) fn input_button(element: ElementRef<'_>) -> bool {
    element.value().name() == "input"
        && element.value().attr("type").is_some_and(|kind| {
            matches!(
                kind.to_ascii_lowercase().as_str(),
                "button" | "submit" | "image"
            )
        })
}

fn disabled(element: ElementRef<'_>) -> bool {
    std::iter::once(element)
        .chain(element.ancestors().filter_map(ElementRef::wrap))
        .any(|e| {
            e.value().attr("disabled").is_some()
                || e.value().attr("aria-disabled") == Some("true")
                || e.value()
                    .has_class("disabled", scraper::CaseSensitivity::AsciiCaseInsensitive)
        })
}

fn page_key(key: &str) -> bool {
    matches!(
        key.to_ascii_lowercase().as_str(),
        "page" | "pageno" | "pagenum" | "pageindex" | "page_no"
    )
}

/// The page its query names, as ?page=2.
fn query_page(url: &PageUrl) -> Option<u32> {
    (url.parsed().query_pairs())
        .find_map(|(key, value)| page_key(&key).then(|| value.parse().ok())?)
}

fn same_list_url(current: &PageUrl, target: &PageUrl) -> bool {
    if current.parsed().origin() != target.parsed().origin() {
        return false;
    }
    if current.parsed().path() == target.parsed().path() {
        let page_fields = |url: &PageUrl| {
            url.parsed()
                .query_pairs()
                .filter(|(key, _)| page_key(key))
                .map(|(key, value)| (key.into_owned(), value.parse::<u32>()))
                .collect::<Vec<_>>()
        };
        let current_fields = page_fields(current);
        let target_fields = page_fields(target);
        if current_fields.len() > 1
            || target_fields.len() > 1
            || current_fields
                .iter()
                .chain(&target_fields)
                .any(|(_, value)| value.is_err())
            || current_fields
                .first()
                .zip(target_fields.first())
                .is_some_and(|(a, b)| a.0 != b.0)
        {
            return false;
        }
        let other_query = |url: &PageUrl| {
            url.parsed()
                .query_pairs()
                .filter(|(key, _)| !page_key(key))
                .map(|(k, v)| (k.into_owned(), v.into_owned()))
                .collect::<Vec<_>>()
        };
        return current != target
            && (query_page(current).is_some() || query_page(target).is_some())
            && other_query(current) == other_query(target);
    }
    let current_path = current.parsed().path();
    let target_path = target.parsed().path();
    let mut common = current_path
        .chars()
        .zip(target_path.chars())
        .take_while(|(a, b)| a == b)
        .map(|(c, _)| c.len_utf8())
        .sum::<usize>();
    while common > 0 && current_path.as_bytes()[common - 1].is_ascii_digit() {
        common -= 1;
    }
    let current_tail = current_path[common..].trim_start_matches(['_', '-']);
    let target_tail = target_path[common..].trim_start_matches(['_', '-']);
    let current_rest = current_tail.trim_start_matches(|c: char| c.is_ascii_digit());
    let target_rest = target_tail.trim_start_matches(|c: char| c.is_ascii_digit());
    (current_rest != current_tail || target_rest != target_tail)
        && current_rest == target_rest
        && current.parsed().query() == target.parsed().query()
}

fn marked_container(element: ElementRef<'_>) -> bool {
    ["class", "id", "aria-label"]
        .iter()
        .filter_map(|a| element.value().attr(a))
        .any(|s| {
            let s = s.to_lowercase();
            [
                "pagination",
                "paging",
                "pager",
                "pagenav",
                "page-nav",
                "page_nav",
                "pagebar",
                "page navigation",
                "翻页",
                "分页",
            ]
            .iter()
            .any(|cue| s.contains(cue))
                || s.split_whitespace().any(|s| s == "pages")
        })
}

struct Context<'a> {
    element: ElementRef<'a>,
    recognized: bool,
    consecutive: bool,
    /// The page it is on and how many it counts.
    counts: Option<(u32, u32)>,
}

static CURRENT: LazyLock<Selector> = LazyLock::new(|| {
    Selector::parse(".curr_page, .current_page, .current-page, .current, [aria-current='page']")
        .unwrap_or_else(|_| unreachable!("fixed current page selector is valid"))
});
static TOTAL: LazyLock<Selector> = LazyLock::new(|| {
    Selector::parse(".all_pages, .total_pages, .total-pages")
        .unwrap_or_else(|_| unreachable!("fixed total pages selector is valid"))
});
static PAGE_COUNTS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?:第\s*)?(\d+)\s*(?:页\s*)?/\s*(?:共\s*)?(\d+)\s*页?")
        .unwrap_or_else(|_| unreachable!("fixed page count pattern is valid"))
});
/// The page a pager is on and how many there are, written apart, as "第 1 页 共 5 页".
static PAGE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"第\s*(\d+)\s*页").unwrap_or_else(|_| unreachable!("fixed page pattern is valid"))
});
static PAGES: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"共\s*(\d+)\s*页").unwrap_or_else(|_| unreachable!("fixed pages pattern is valid"))
});

pub(crate) fn extract(controls: &[Control<'_>], url: &PageUrl, has_scripts: bool) -> Pagination {
    let mut contexts: Vec<Context<'_>> = Vec::new();
    let mut pagination = Pagination::default();
    // Whether the pager leads on to a later page, by a link or a click.
    let mut later = false;
    // A last-page control leading here says this is the last page, which another pager on it, as
    // one for phones, can still offer a next page past.
    let at_last =
        (controls.iter()).any(|c| matches!(c.kind, Kind::Last) && c.link.as_ref() == Some(url));
    for control in controls {
        if disabled(control.element) {
            continue;
        }
        // The nearest pager the control is in, and the nearest of its own that counts the pages:
        // a pager can count them beside its controls, as "页码：1/1" after them. Labelled
        // otherwise, a control is a pager's only as a link naming a page.
        let named = control.link.as_ref().and_then(query_page);
        let (mut context, mut counted) = (None, None);
        for element in control
            .element
            .ancestors()
            .filter_map(ElementRef::wrap)
            .take_while(|e| !matches!(e.value().name(), "body" | "html"))
            .take_while(|_| !matches!(control.kind, Kind::Other) || named.is_some())
        {
            let index =
                if let Some(index) = contexts.iter().position(|c| c.element.id() == element.id()) {
                    index
                } else {
                    let members: Vec<_> = controls
                        .iter()
                        .filter(|c| c.element.ancestors().any(|n| n.id() == element.id()))
                        .collect();
                    let group = members.iter().all(|c| !matches!(c.kind, Kind::Other));
                    let mut numbers: Vec<_> = members
                        .iter()
                        .filter_map(|c| match c.kind {
                            Kind::Number(n) => Some(n),
                            _ => None,
                        })
                        .collect();
                    let consecutive_links = group
                        && numbers
                            .windows(2)
                            .any(|pair| pair[0].checked_add(1) == Some(pair[1]));
                    let number = |selector: &Selector| {
                        element
                            .select(selector)
                            .next()
                            .and_then(|e| visible_text(e).parse::<u32>().ok())
                    };
                    let marked = marked_container(element);
                    let current = number(&CURRENT);
                    let counts = |text: &str| {
                        let found = |pattern: &Regex| pattern.captures(text)?[1].parse().ok();
                        let page = current.or_else(|| found(&PAGE));
                        page.zip(number(&TOTAL).or_else(|| found(&PAGES)))
                            .or_else(|| {
                                let found = PAGE_COUNTS.captures(text)?;
                                Some((found[1].parse().ok()?, found[2].parse().ok()?))
                            })
                            .filter(|&(page, _)| current.is_none_or(|c| c == page))
                    };
                    // Read apart, a pager's "1" beside its "1/3" is not "11/3", but a number drawn
                    // in parts, as "1" and "0" of 10, is not 1. A reading past its own count is
                    // none, and so is one of a page other than that marked current; two others
                    // differing tell nothing, as "3/3" beside the 50 rows a page shows reads
                    // "3/350" together.
                    let counts = (marked || group)
                        .then(|| {
                            let [apart, joined] = [visible_words(element), visible_text(element)]
                                .map(|text| counts(&text).filter(|(page, total)| page <= total));
                            match (apart, joined) {
                                (Some(apart), Some(joined)) => (apart == joined).then_some(apart),
                                (apart, joined) => apart.or(joined),
                            }
                        })
                        .flatten();
                    let recognized = marked || consecutive_links || counts.is_some();
                    if recognized {
                        numbers.extend(
                            element
                                .select(&CURRENT)
                                .filter_map(|e| visible_text(e).parse::<u32>().ok()),
                        );
                        numbers.sort_unstable();
                        numbers.dedup();
                    }
                    let consecutive = recognized
                        && numbers
                            .windows(2)
                            .any(|pair| pair[0].checked_add(1) == Some(pair[1]));
                    contexts.push(Context {
                        element,
                        recognized,
                        consecutive,
                        counts,
                    });
                    contexts.len() - 1
                };
            if contexts[index].recognized {
                context.get_or_insert(index);
                if contexts[index].counts.is_some() {
                    counted = Some(index);
                    break;
                }
            } else if context.is_some() {
                break;
            }
        }
        let context = context.map(|i| &contexts[i]);
        let counted = counted.and_then(|i| contexts[i].counts);
        pagination.page = pagination.page.or(counted.map(|(page, _)| page));
        let is_page = match control.kind {
            Kind::Forward | Kind::Last | Kind::Backward | Kind::Jump => true,
            Kind::First => {
                context.is_some()
                    || control
                        .link
                        .as_ref()
                        .is_some_and(|link| same_list_url(url, link))
            }
            Kind::Number(_) => {
                context.is_some_and(|c| c.consecutive)
                    || control
                        .link
                        .as_ref()
                        .is_some_and(|link| same_list_url(url, link))
            }
            Kind::Other => named.is_some(),
        };
        if !is_page
            || (matches!(control.kind, Kind::Forward | Kind::Last | Kind::Jump)
                && (at_last || counted.is_some_and(|(page, total)| total > 0 && page >= total)))
        {
            continue;
        }
        let onward = match control.kind {
            Kind::Backward | Kind::First => false,
            Kind::Number(n) => counted.is_none_or(|(page, _)| n > page),
            // Labelled otherwise, as 返回, a link leads on only to a page after the one this is, as
            // its address names it or else the pager the link is in counts it: a query counting
            // from 0 falls one short of the pager, which at most notes pages unread that were not.
            Kind::Other => {
                let on = query_page(url).or(counted.map(|(page, _)| page));
                named.zip(on).is_some_and(|(n, on)| n > on)
            }
            _ => true,
        };
        if let Some(link) = &control.link {
            if link.website() == url.website() && link != url {
                if !pagination.urls.contains(link) {
                    pagination.urls.push(link.clone());
                }
                later |= onward;
            }
        } else if matches!(control.kind, Kind::Jump) {
            // Past the check for its last page above, a pager counting its pages has more.
            pagination.jump_only |= counted.is_some();
        } else if !matches!(control.kind, Kind::Backward | Kind::First)
            && scripted(control.element, has_scripts)
        {
            pagination.needs_browser = true;
            let page = match control.kind {
                Kind::Number(n) => Some(n),
                Kind::Last => counted.map(|(_, total)| total),
                _ => None,
            };
            if !matches!(control.kind, Kind::Last) || page.is_some() {
                if !pagination.buttons.iter().any(|b| b.control == control.text) {
                    pagination.buttons.push(crate::page::ButtonPage {
                        page,
                        control: control.text.clone(),
                    });
                }
                later |= onward;
            }
        }
    }
    // A jump goes to the page written in the box beside it, which no click replays: the pages
    // only it leads to stay unread, though the pager can lead back to earlier ones.
    pagination.jump_only &= !later;
    pagination
}

/// The labels of controls that lead nowhere by themselves but that the page's scripts may act on,
/// such as the categories of a list loading each when clicked. Page turns are left out, and so are
/// labels of more than one such control, as a click finds its control by the label among them: a
/// link of the same label, as in a site's menu, is not clicked.
pub(crate) fn clickable(controls: &[Control<'_>], has_scripts: bool) -> Vec<String> {
    controls
        .iter()
        .filter(|c| {
            c.link.is_none()
                && matches!(c.kind, Kind::Other)
                && !c.text.is_empty()
                && c.text != crate::page::UNLABELED
                // Longer is a sentence or a whole card rather than a name.
                && c.text.chars().count() <= 20
                // A number is a part's place, as a carousel dot's, not what the part shows.
                && c.text.parse::<u32>().is_err()
                && !disabled(c.element)
                && scripted(c.element, has_scripts)
                && (controls.iter())
                    .filter(|other| other.link.is_none() && other.text == c.text)
                    .count()
                    == 1
        })
        .map(|c| c.text.clone())
        .collect()
}

/// Leads nowhere by itself, so that only a script acts on it.
fn scripted(element: ElementRef<'_>, has_scripts: bool) -> bool {
    let value = element.value();
    value.attr("onclick").is_some()
        || match value.attr("href") {
            Some(href) => javascript_control(href, has_scripts),
            None => value.name() == "a" && has_scripts,
        }
        || value.name() == "button"
        || input_button(element)
        || value.attr("role") == Some("button")
}

fn javascript_control(href: &str, has_scripts: bool) -> bool {
    let href: String = href
        .chars()
        .filter(|c| !c.is_whitespace() && *c != ';')
        .flat_map(char::to_lowercase)
        .collect();
    href.strip_prefix("javascript:").is_some_and(|script| {
        has_scripts || !matches!(script, "" | "void(0)" | "void0" | "undefined")
    })
}
