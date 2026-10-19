//! Escaped HTML fragments: KPIs, notices, panels, tables, forms and
//! navigation.
//!
//! Every dynamic string passes [`html_escape`]; attribute values pass
//! [`attr`]. Parameters named `body_html`/`control_html` carry fragments
//! produced by this crate (they are inserted as markup on purpose); every
//! caller-supplied text is escaped.
//!
//! Links are only honored for safe destinations: relative paths, fragments,
//! `http://`, `https://` and `mailto:`. A destination with any other scheme
//! (`javascript:`, `data:`) renders its label as plain text instead of a
//! live link.

use crate::escape::{attr, html_escape, push_escaped};
use crate::status::{chip, StatusKind};

/// A KPI card: a small label, a large value, a note and an optional link.
pub fn kpi_card(label: &str, value: &str, note: &str, href: Option<&str>) -> String {
    let value_html = match href {
        Some(href) if safe_href(href) => format!(
            "<a class=\"kpi-link\" href=\"{}\">{}</a>",
            attr(href),
            html_escape(value)
        ),
        _ => html_escape(value),
    };
    format!(
        "<div class=\"kpi\"><p class=\"kpi-label\">{}</p><p class=\"kpi-value\">{value_html}</p>\
         <p class=\"kpi-note\">{}</p></div>",
        html_escape(label),
        html_escape(note)
    )
}

/// A responsive row of KPI cards. `cards` carry fragments produced by
/// [`kpi_card`]; they are inserted as markup on purpose.
pub fn kpi_row(cards: &[String]) -> String {
    format!("<div class=\"kpi-row\">{}</div>", cards.join(""))
}

/// A two-column layout: the main fragment beside a side fragment, stacking to
/// one column below 900 CSS pixels. Both fragments are inserted as markup.
pub fn split(main_html: &str, side_html: &str) -> String {
    format!("<div class=\"split\"><div>{main_html}</div><div>{side_html}</div></div>")
}

/// A notice banner. `Neutral` renders the pale cobalt info variant,
/// `Attention` the citron tint with warning text, `Failure` the pale danger
/// variant; every other kind renders as info. `action` is an optional
/// `(label, href)`.
pub fn notice(kind: StatusKind, title: &str, body: &str, action: Option<(&str, &str)>) -> String {
    let (variant, role) = match kind {
        StatusKind::Attention => ("notice-attention", "status"),
        StatusKind::Failure => ("notice-failure", "alert"),
        _ => ("notice-info", "status"),
    };
    let mut out = format!(
        "<div class=\"notice {variant}\" role=\"{role}\">\
         <p class=\"notice-title\">{}</p><p class=\"notice-body\">{}</p>",
        html_escape(title),
        html_escape(body)
    );
    if let Some((label, href)) = action {
        out.push_str("<p class=\"notice-action\">");
        out.push_str(&action_label(label, href, false));
        out.push_str("</p>");
    }
    out.push_str("</div>");
    out
}

/// A panel: one heading, one purpose, an optional next step.
pub fn panel(heading: &str, body_html: &str, next_step: Option<&str>) -> String {
    let mut out = format!(
        "<section class=\"panel\"><h2 class=\"panel-heading\">{}</h2>\
         <div class=\"panel-body\">{body_html}</div>",
        html_escape(heading)
    );
    if let Some(next) = next_step {
        out.push_str(&format!(
            "<p class=\"panel-next\">Next step: {}</p>",
            html_escape(next)
        ));
    }
    out.push_str("</section>");
    out
}

/// One table cell.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Cell {
    /// Escaped text.
    Text(String),
    /// Monospace text for exact identifiers and raw values.
    Mono(String),
    /// A numeric value, right-aligned with tabular figures.
    Num(String),
    /// A status chip with its label and kind.
    Status(String, StatusKind),
    /// A link; unsafe destinations render as plain text.
    Link(String, String),
    /// De-emphasized supporting text.
    Muted(String),
}

/// The empty state shown instead of an empty table body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmptyState {
    /// The reason the table is empty.
    pub reason: String,
    /// An optional `(label, href)` next action.
    pub action: Option<(String, String)>,
}

/// A real `<table>` with `<th scope="col">` headers, an optional caption and
/// a row count note. When `rows` is empty and `empty` is given, the empty
/// state replaces the table body.
pub fn data_table(
    caption: Option<&str>,
    headers: &[&str],
    rows: &[Vec<Cell>],
    empty: Option<EmptyState>,
    count_note: Option<&str>,
) -> String {
    if rows.is_empty() {
        if let Some(empty) = empty {
            return empty_table(caption, headers, &empty, count_note);
        }
    }
    // One reservation sized from the cells themselves. A 20k-row table would
    // otherwise grow the buffer by repeated doubling (and build a temporary
    // String per cell); this keeps the render linear and allocation-light.
    let mut out = String::with_capacity(table_capacity(caption, headers, rows, count_note));
    out.push_str("<div class=\"table-wrap\"><table class=\"data-table\">");
    if let Some(caption) = caption {
        out.push_str("<caption class=\"table-caption\">");
        push_escaped(&mut out, caption);
        out.push_str("</caption>");
    }
    out.push_str("<thead><tr>");
    for header in headers {
        out.push_str("<th scope=\"col\">");
        push_escaped(&mut out, header);
        out.push_str("</th>");
    }
    out.push_str("</tr></thead><tbody>");
    if rows.is_empty() {
        out.push_str("<tr><td colspan=\"");
        out.push_str(&headers.len().to_string());
        out.push_str("\" class=\"cell-muted\">");
        out.push_str(&html_escape(&crate::format::or_not_available("no rows")));
        out.push_str("</td></tr>");
    }
    for row in rows {
        out.push_str("<tr>");
        for cell in row {
            push_cell(&mut out, cell);
        }
        out.push_str("</tr>");
    }
    out.push_str("</tbody></table></div>");
    out.push_str(&note_line(count_note));
    out
}

/// The bytes [`data_table`] will emit (fixed markup plus the cells' text),
/// used to reserve the output once. Escaped characters can add bytes, so the
/// callers that need the exact bound pass plain text; the reserve is a lower
/// bound, never a truncation.
fn table_capacity(
    caption: Option<&str>,
    headers: &[&str],
    rows: &[Vec<Cell>],
    count_note: Option<&str>,
) -> usize {
    let mut capacity = 49 + 30 + 21;
    capacity += caption.map_or(0, |caption| 41 + caption.len());
    capacity += headers
        .iter()
        .map(|header| 22 + header.len())
        .sum::<usize>();
    capacity += count_note.map_or(0, |note| 26 + note.len());
    for row in rows {
        capacity += 9;
        for cell in row {
            capacity += cell_capacity(cell);
        }
    }
    if rows.is_empty() {
        capacity += 64;
    }
    capacity
}

fn cell_capacity(cell: &Cell) -> usize {
    match cell {
        Cell::Text(text) => 9 + text.len(),
        Cell::Mono(text) => 40 + text.len(),
        Cell::Num(text) => 26 + text.len(),
        Cell::Status(label, kind) => 51 + kind.class().len() + label.len(),
        Cell::Link(text, href) => {
            if safe_href(href) {
                24 + href.len() + text.len()
            } else {
                9 + text.len()
            }
        }
        Cell::Muted(text) => 28 + text.len(),
    }
}

fn table_head(caption: Option<&str>, headers: &[&str]) -> String {
    let mut out = String::from("<div class=\"table-wrap\"><table class=\"data-table\">");
    if let Some(caption) = caption {
        out.push_str(&format!(
            "<caption class=\"table-caption\">{}</caption>",
            html_escape(caption)
        ));
    }
    out.push_str("<thead><tr>");
    for header in headers {
        out.push_str(&format!("<th scope=\"col\">{}</th>", html_escape(header)));
    }
    out.push_str("</tr></thead></table></div>");
    out
}

/// The empty variant of [`data_table`]: headers stay (so the reader sees the
/// columns) and the body is replaced by the empty state with its reason.
fn empty_table(
    caption: Option<&str>,
    headers: &[&str],
    empty: &EmptyState,
    count_note: Option<&str>,
) -> String {
    let mut out = table_head(caption, headers);
    out.push_str("<div class=\"empty-state\">");
    out.push_str(&format!(
        "<p class=\"empty-reason\">{}</p>",
        html_escape(&empty.reason)
    ));
    if let Some((label, href)) = &empty.action {
        out.push_str(&action(label, href, false));
    }
    out.push_str("</div>");
    out.push_str(&note_line(count_note));
    out
}

fn note_line(note: Option<&str>) -> String {
    match note {
        Some(note) => format!("<p class=\"table-note\">{}</p>", html_escape(note)),
        None => String::new(),
    }
}

fn push_cell(out: &mut String, cell: &Cell) {
    match cell {
        Cell::Text(text) => {
            out.push_str("<td>");
            push_escaped(out, text);
            out.push_str("</td>");
        }
        Cell::Mono(text) => {
            out.push_str("<td class=\"cell-mono\"><code>");
            push_escaped(out, text);
            out.push_str("</code></td>");
        }
        Cell::Num(text) => {
            out.push_str("<td class=\"cell-num\">");
            push_escaped(out, text);
            out.push_str("</td>");
        }
        Cell::Status(label, kind) => {
            out.push_str("<td class=\"cell-status\">");
            out.push_str(&chip(label, *kind));
            out.push_str("</td>");
        }
        Cell::Link(text, href) => {
            if safe_href(href) {
                out.push_str("<td><a href=\"");
                push_escaped(out, href);
                out.push_str("\">");
                push_escaped(out, text);
                out.push_str("</a></td>");
            } else {
                out.push_str("<td>");
                push_escaped(out, text);
                out.push_str("</td>");
            }
        }
        Cell::Muted(text) => {
            out.push_str("<td class=\"cell-muted\">");
            push_escaped(out, text);
            out.push_str("</td>");
        }
    }
}

#[cfg(test)]
fn cell_html(cell: &Cell) -> String {
    let mut out = String::new();
    push_cell(&mut out, cell);
    out
}

/// A form field: persistent label, optional help and error text, and the
/// control. The control receives `aria-describedby` (pointing at the help
/// and error ids) and `aria-invalid` when an error is present.
pub fn field(
    id: &str,
    label: &str,
    help: Option<&str>,
    error: Option<&str>,
    control_html: &str,
) -> String {
    let mut described_by: Vec<String> = Vec::new();
    let mut out = String::new();
    let invalid = error.is_some();
    out.push_str(&format!(
        "<div class=\"field{}\">",
        if invalid { " field-invalid" } else { "" }
    ));
    out.push_str(&format!(
        "<label class=\"field-label\" for=\"{}\">{}</label>",
        attr(id),
        html_escape(label)
    ));
    if let Some(help) = help {
        described_by.push(format!("{id}-help"));
        out.push_str(&format!(
            "<p class=\"field-help\" id=\"{}-help\">{}</p>",
            attr(id),
            html_escape(help)
        ));
    }
    if let Some(error) = error {
        described_by.push(format!("{id}-error"));
        out.push_str(&format!(
            "<p class=\"field-error\" id=\"{}-error\">{}</p>",
            attr(id),
            html_escape(error)
        ));
    }
    out.push_str(&inject_control_attrs(
        control_html,
        &described_by.join(" "),
        invalid,
    ));
    out.push_str("</div>");
    out
}

/// Insert `aria-describedby`/`aria-invalid` into the first control tag of a
/// fragment produced by this crate, so the label wiring holds without a
/// template engine.
fn inject_control_attrs(control_html: &str, described_by: &str, invalid: bool) -> String {
    let mut injection = String::new();
    if !described_by.is_empty() {
        injection.push_str(&format!(" aria-describedby=\"{}\"", attr(described_by)));
    }
    if invalid {
        injection.push_str(" aria-invalid=\"true\"");
    }
    if injection.is_empty() {
        return control_html.to_string();
    }
    let mut earliest: Option<(usize, usize)> = None;
    for tag in ["<input", "<select", "<textarea"] {
        if let Some(position) = control_html.find(tag) {
            let after = position + tag.len();
            let next = control_html.as_bytes().get(after).copied();
            let is_tag_start = matches!(next, Some(b' ' | b'>' | b'/') | None);
            if is_tag_start && earliest.map(|(p, _)| position < p).unwrap_or(true) {
                earliest = Some((position, tag.len()));
            }
        }
    }
    match earliest {
        Some((position, tag_len)) => {
            let split = position + tag_len;
            format!(
                "{}{injection}{}",
                &control_html[..split],
                &control_html[split..]
            )
        }
        None => control_html.to_string(),
    }
}

/// A text-like input control.
pub fn input(
    id: &str,
    name: &str,
    kind: &str,
    value: Option<&str>,
    required: bool,
    autocomplete: Option<&str>,
) -> String {
    let mut out = format!(
        "<input class=\"input\" type=\"{}\" id=\"{}\" name=\"{}\"",
        attr(kind),
        attr(id),
        attr(name)
    );
    if let Some(value) = value {
        out.push_str(&format!(" value=\"{}\"", attr(value)));
    }
    if required {
        out.push_str(" required");
    }
    if let Some(autocomplete) = autocomplete {
        out.push_str(&format!(" autocomplete=\"{}\"", attr(autocomplete)));
    }
    out.push('>');
    out
}

/// A textarea control.
pub fn textarea(
    id: &str,
    name: &str,
    value: Option<&str>,
    rows: usize,
    maxlength: Option<usize>,
) -> String {
    let mut out = format!(
        "<textarea class=\"input\" id=\"{}\" name=\"{}\" rows=\"{rows}\"",
        attr(id),
        attr(name)
    );
    if let Some(maxlength) = maxlength {
        out.push_str(&format!(" maxlength=\"{maxlength}\""));
    }
    out.push('>');
    if let Some(value) = value {
        out.push_str(&html_escape(value));
    }
    out.push_str("</textarea>");
    out
}

/// A select control over `(value, label)` options; `selected` matches an
/// option value.
pub fn select(
    id: &str,
    name: &str,
    options: &[(String, String)],
    selected: Option<&str>,
) -> String {
    let mut out = format!(
        "<select class=\"input\" id=\"{}\" name=\"{}\">",
        attr(id),
        attr(name)
    );
    for (value, label) in options {
        let selected_attr = if selected == Some(value.as_str()) {
            " selected"
        } else {
            ""
        };
        out.push_str(&format!(
            "<option value=\"{}\"{selected_attr}>{}</option>",
            attr(value),
            html_escape(label)
        ));
    }
    out.push_str("</select>");
    out
}

/// A checkbox with its label enclosed (so the control is labelled by
/// containment, which holds even without an id).
pub fn checkbox(id: &str, name: &str, label: &str, checked: bool) -> String {
    format!(
        "<label class=\"checkbox\"><input type=\"checkbox\" id=\"{}\" name=\"{}\"{}> {}</label>",
        attr(id),
        attr(name),
        if checked { " checked" } else { "" },
        html_escape(label)
    )
}

/// A form error summary: `role="alert"` with one link per errored field,
/// pointing at the field's id. Entries with an empty message are skipped; if
/// nothing remains the summary is empty.
pub fn form_error_summary(errors: &[(String, String)]) -> String {
    let mut items = String::new();
    for (field_id, message) in errors {
        if message.trim().is_empty() {
            continue;
        }
        items.push_str(&format!(
            "<li><a href=\"#{}\">{}</a></li>",
            attr(field_id),
            html_escape(message)
        ));
    }
    if items.is_empty() {
        return String::new();
    }
    format!(
        "<div class=\"form-errors\" role=\"alert\"><ul class=\"form-errors-list\">{items}</ul></div>"
    )
}

/// A native details element for progressive technical detail.
pub fn details(summary: &str, body_html: &str, open: bool) -> String {
    format!(
        "<details class=\"details\"{}><summary>{}</summary>\
         <div class=\"details-body\">{body_html}</div></details>",
        if open { " open" } else { "" },
        html_escape(summary)
    )
}

/// A GET filter form: one labelled text input per `(name, value)` field, a
/// submit action, an optional clear link and the result note under the form.
pub fn filter_bar(fields: &[(String, String)], clear_href: &str, result_note: &str) -> String {
    let mut out = String::from("<form class=\"filter-bar\" method=\"get\">");
    for (name, value) in fields {
        let field_id = format!("filter-{}", id_slug(name));
        let label = humanize(name);
        out.push_str(&format!(
            "<div class=\"filter-field\"><label class=\"field-label\" for=\"{}\">{}</label>\
             <input class=\"input\" type=\"text\" id=\"{}\" name=\"{}\" value=\"{}\"></div>",
            attr(&field_id),
            html_escape(&label),
            attr(&field_id),
            attr(name),
            attr(value)
        ));
    }
    out.push_str("<button class=\"action action-primary\" type=\"submit\">Apply filters</button>");
    if !clear_href.trim().is_empty() && safe_href(clear_href) {
        out.push_str(&format!(
            "<a class=\"filter-clear\" href=\"{}\">Clear filters</a>",
            attr(clear_href)
        ));
    }
    out.push_str("</form>");
    out.push_str(&format!(
        "<p class=\"table-note\">{}</p>",
        html_escape(result_note)
    ));
    out
}

/// Tabs over stable GET URLs: one link per `(label, href, active)` item with
/// `aria-current="page"` on the active tab.
pub fn tabs(items: &[(String, String, bool)]) -> String {
    let mut out = String::from("<nav class=\"tabs\" aria-label=\"Sections\">");
    for (label, href, active) in items {
        let current = if *active {
            " aria-current=\"page\""
        } else {
            ""
        };
        out.push_str(&format!(
            "<a class=\"tab\"{current} href=\"{}\">{}</a>",
            attr(href),
            html_escape(label)
        ));
    }
    out.push_str("</nav>");
    out
}

/// Pagination over stable GET URLs with a result note between the links.
pub fn pagination(prev: Option<&str>, next: Option<&str>, note: &str) -> String {
    let mut out = String::from("<nav class=\"pagination\" aria-label=\"Pagination\">");
    if let Some(prev) = prev {
        out.push_str(&format!(
            "<a class=\"pagination-link\" rel=\"prev\" href=\"{}\">Previous</a>",
            attr(prev)
        ));
    }
    out.push_str(&format!(
        "<span class=\"pagination-note\">{}</span>",
        html_escape(note)
    ));
    if let Some(next) = next {
        out.push_str(&format!(
            "<a class=\"pagination-link\" rel=\"next\" href=\"{}\">Next</a>",
            attr(next)
        ));
    }
    out.push_str("</nav>");
    out
}

/// A breadcrumb trail; the last part is the current page.
pub fn breadcrumb(parts: &[String]) -> String {
    if parts.is_empty() {
        return String::new();
    }
    let last = parts.len() - 1;
    let mut out = String::from(
        "<nav class=\"breadcrumb\" aria-label=\"Breadcrumb\"><ol class=\"breadcrumb-list\">",
    );
    for (index, part) in parts.iter().enumerate() {
        let current = if index == last {
            " aria-current=\"page\""
        } else {
            ""
        };
        out.push_str(&format!(
            "<li class=\"breadcrumb-item\"{current}>{}</li>",
            html_escape(part)
        ));
    }
    out.push_str("</ol></nav>");
    out
}

/// A primary or secondary link action. An unsafe destination renders the
/// label as plain text instead of a link.
pub fn action(label: &str, href: &str, primary: bool) -> String {
    action_label(label, href, primary)
}

fn action_label(label: &str, href: &str, primary: bool) -> String {
    let class = if primary {
        "action action-primary"
    } else {
        "action action-secondary"
    };
    if safe_href(href) {
        format!(
            "<a class=\"{class}\" href=\"{}\">{}</a>",
            attr(href),
            html_escape(label)
        )
    } else {
        format!("<span class=\"{class}\">{}</span>", html_escape(label))
    }
}

/// A submit button action carrying `name=value` (used for one clear submit
/// action inside a POST form).
pub fn action_button(label: &str, name: &str, value: &str, primary: bool) -> String {
    let class = if primary {
        "action action-primary"
    } else {
        "action action-secondary"
    };
    format!(
        "<button class=\"{class}\" type=\"submit\" name=\"{}\" value=\"{}\">{}</button>",
        attr(name),
        attr(value),
        html_escape(label)
    )
}

/// The CSRF hidden input for a POST form.
pub fn csrf_hidden(token: &str) -> String {
    format!(
        "<input type=\"hidden\" name=\"csrf_token\" value=\"{}\">",
        attr(token)
    )
}

/// A label and the standard status chip for the given kind, so a status
/// always includes a text label.
pub fn status_pair(label: &str, kind: StatusKind) -> String {
    format!(
        "<span class=\"status-pair\"><span class=\"status-pair-label\">{}</span>{}</span>",
        html_escape(label),
        chip(kind.label(), kind)
    )
}

/// Destinations a rendered link may use: relative paths and fragments,
/// `http(s)://` and `mailto:`. Protocol-relative (`//host`) and every other
/// scheme are rejected so `javascript:` and `data:` payloads never become
/// live links.
fn safe_href(href: &str) -> bool {
    let href = href.trim();
    if href.is_empty() || href.starts_with("//") {
        return false;
    }
    if href.starts_with('/')
        || href.starts_with('#')
        || href.starts_with("./")
        || href.starts_with("../")
    {
        return true;
    }
    if has_prefix_ignore_ascii_case(href, "https://")
        || has_prefix_ignore_ascii_case(href, "http://")
        || has_prefix_ignore_ascii_case(href, "mailto:")
    {
        return true;
    }
    // A relative path has no scheme separator before its first slash.
    match (href.find(':'), href.find('/')) {
        (Some(colon), Some(slash)) => slash < colon,
        (Some(_), None) => false,
        (None, _) => true,
    }
}

/// Whether `value` starts with `prefix`, ignoring ASCII case, without
/// allocating a lowercase copy (called once per link cell).
fn has_prefix_ignore_ascii_case(value: &str, prefix: &str) -> bool {
    value
        .as_bytes()
        .get(..prefix.len())
        .is_some_and(|head| head.eq_ignore_ascii_case(prefix.as_bytes()))
}

/// An id-safe slug for a filter field's derived element id.
fn id_slug(name: &str) -> String {
    name.chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '-' })
        .collect()
}

/// A human label for a filter field name (`last_30_days` to
/// `Last 30 days`).
fn humanize(name: &str) -> String {
    let spaced: String = name
        .chars()
        .map(|ch| if ch == '_' || ch == '-' { ' ' } else { ch })
        .collect();
    let trimmed = spaced.trim();
    let mut chars = trimmed.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOSTILE: &str = "<script>alert(\"x\")</script>&'";

    fn assert_escaped(html: &str) {
        assert!(!html.contains("<script"), "raw script tag leaked: {html}");
        assert!(!html.contains("alert(\"x\")"), "raw payload leaked: {html}");
        assert!(
            !html.contains("</script>"),
            "raw closing tag leaked: {html}"
        );
    }

    /// Rendering is one row and one cell per input, at the protocol's
    /// 10_000-row reference size: a duplicated or re-scanned render (the
    /// shape a quadratic table builder takes) would show up as extra rows,
    /// cells or repeated cell text.
    #[test]
    fn data_table_renders_every_row_and_cell_exactly_once_at_ten_thousand_rows() {
        let rows: Vec<Vec<Cell>> = (0..10_000)
            .map(|i| {
                vec![
                    Cell::Mono(format!("t{i:016x}")),
                    Cell::Text(format!("lead{i}@p{i}.example")),
                    Cell::Num(format!("{i}")),
                ]
            })
            .collect();
        let headers = ["Touch", "Contact", "N"];
        let html = data_table(Some("Rows"), &headers, &rows, None, None);
        assert_eq!(
            html.matches("<tr").count(),
            10_001,
            "one row per input, plus the header"
        );
        assert_eq!(html.matches("<td").count(), 30_000, "one cell per input");
        for probe in [
            "t0000000000000000",
            "lead0@p0.example",
            "t000000000000270f", // row 9_999 in hex
            "lead9999@p9999.example",
        ] {
            assert_eq!(
                html.matches(probe).count(),
                1,
                "{probe} must appear exactly once"
            );
        }
    }

    /// The 10k-row render reserves its output once: the buffer's capacity is
    /// the table's own estimate (plus allocator slack), not the power-of-two
    /// growth a `String::new()` builder would show. The same test pins output
    /// bytes at n/2n/4n as linear.
    #[test]
    fn data_table_reserves_once_and_stays_linear_at_twenty_thousand_rows() {
        fn rows(count: usize) -> Vec<Vec<Cell>> {
            (0..count)
                .map(|i| {
                    vec![
                        Cell::Mono(format!("t{i:016x}")),
                        Cell::Text(format!("lead{i}@p{i}.example")),
                        Cell::Num(format!("{i}")),
                        Cell::Status("Verified".to_string(), StatusKind::Verified),
                    ]
                })
                .collect()
        }
        let headers = ["Touch", "Contact", "N", "Status"];
        let render = |count: usize| {
            let rows = rows(count);
            let estimate = table_capacity(Some("Rows"), &headers, &rows, Some("count"));
            let html = data_table(Some("Rows"), &headers, &rows, None, Some("count"));
            (html, estimate)
        };

        let (five, _) = render(5_000);
        let (ten, estimate) = render(10_000);
        let (twenty, _) = render(20_000);
        assert_eq!(twenty.matches("<tr").count(), 20_001);
        assert_eq!(twenty.matches("<td").count(), 80_000);
        assert!(
            ten.capacity() >= ten.len(),
            "the reserve covers the rendered markup"
        );
        assert!(
            ten.capacity() <= estimate + 4096,
            "one reserve sized from the cells (capacity {}, estimate {}, len {})",
            ten.capacity(),
            estimate,
            ten.len()
        );
        // Per-row bytes stay bounded as the row count doubles (the row text
        // itself grows with the index, so compare per row rather than total).
        let per_row = |html: &str, rows: usize| html.len() / rows;
        assert!(
            per_row(&ten, 10_000) <= per_row(&five, 5_000) + 8,
            "5k -> 10k rows is linear per row"
        );
        assert!(
            per_row(&twenty, 20_000) <= per_row(&ten, 10_000) + 8,
            "10k -> 20k rows is linear per row"
        );
    }

    #[test]
    fn kpi_row_and_split_wrap_fragments() {
        let row = kpi_row(&[
            kpi_card("Findings", "5", "Snapshot", None),
            kpi_card("Verified records", "24", "History", None),
        ]);
        assert!(row.starts_with("<div class=\"kpi-row\">"));
        assert_eq!(row.matches("class=\"kpi\"").count(), 2);
        let two = split("<p>main</p>", "<p>side</p>");
        assert!(
            two.contains("<div class=\"split\"><div><p>main</p></div><div><p>side</p></div></div>")
        );
    }

    #[test]
    fn kpi_card_escapes_and_links() {
        let html = kpi_card("Findings", "5", "Snapshot", Some("/console/audit"));
        assert!(html.contains("class=\"kpi\""));
        assert!(html.contains("class=\"kpi-label\">Findings<"));
        assert!(html.contains("<a class=\"kpi-link\" href=\"/console/audit\">5</a>"));
        let hostile = kpi_card(HOSTILE, HOSTILE, HOSTILE, Some("javascript:alert(1)"));
        assert_escaped(&hostile);
        assert!(!hostile.contains("href="), "unsafe href must not render");
        let no_link = kpi_card("L", "V", "N", None);
        assert!(!no_link.contains("<a "));
    }

    #[test]
    fn notice_variants_map_kinds_and_escape() {
        let info = notice(
            StatusKind::Neutral,
            "Reporting is delayed",
            "Two days are not final.",
            None,
        );
        assert!(info.contains("class=\"notice notice-info\""));
        assert!(info.contains("role=\"status\""));
        let attention = notice(StatusKind::Attention, "T", "B", None);
        assert!(attention.contains("notice-attention"));
        let failure = notice(
            StatusKind::Failure,
            "T",
            "B",
            Some(("Contact support", crate::shell::DEFAULT_SUPPORT_URL)),
        );
        assert!(failure.contains("notice-failure"));
        assert!(failure.contains("role=\"alert\""));
        assert!(failure.contains(
            "<a class=\"action action-secondary\" href=\"https://apexmediation.ee/support/\">Contact support</a>"
        ));
        let hostile = notice(
            StatusKind::Failure,
            HOSTILE,
            HOSTILE,
            Some((HOSTILE, "javascript:alert(1)")),
        );
        assert_escaped(&hostile);
        assert!(
            !hostile.contains("href="),
            "unsafe action href must not render"
        );
    }

    #[test]
    fn panel_escapes_heading_and_next_step_but_keeps_body_markup() {
        let html = panel(HOSTILE, "<p>kept</p>", Some(HOSTILE));
        assert!(html.contains("<div class=\"panel-body\"><p>kept</p></div>"));
        assert!(html.contains("Next step: &lt;script&gt;"));
        assert_escaped(&html);
        let no_step = panel("H", "", None);
        assert!(!no_step.contains("panel-next"));
    }

    #[test]
    fn data_table_renders_semantic_table_with_cells() {
        let rows = vec![
            vec![
                Cell::Text("Reporting totals disagree".to_string()),
                Cell::Mono("exp_0247".to_string()),
                Cell::Num("1,250.00".to_string()),
                Cell::Status("Verified".to_string(), StatusKind::Verified),
                Cell::Link("Open".to_string(), "/console/audit".to_string()),
                Cell::Muted("snapshot".to_string()),
            ],
            vec![Cell::Text("second row".to_string())],
        ];
        let html = data_table(
            Some("Findings"),
            &["Finding", "ID", "Amount", "Status", "Action", "Source"],
            &rows,
            None,
            Some("2 findings / as of 14 Oct 2026, 08:10 UTC"),
        );
        assert!(html.contains("<table class=\"data-table\">"));
        assert!(html.contains("<caption class=\"table-caption\">Findings</caption>"));
        assert_eq!(html.matches("<th scope=\"col\">").count(), 6);
        assert!(html.contains("<td class=\"cell-mono\"><code>exp_0247</code></td>"));
        assert!(html.contains("<td class=\"cell-num\">1,250.00</td>"));
        assert!(html.contains(
            "<td class=\"cell-status\"><span class=\"status status-verified\">Verified</span></td>"
        ));
        assert!(html.contains("<td><a href=\"/console/audit\">Open</a></td>"));
        assert!(html.contains("<td class=\"cell-muted\">snapshot</td>"));
        assert!(html.contains("class=\"table-note\">2 findings"));
    }

    #[test]
    fn data_table_keeps_mixed_case_safe_schemes_and_refuses_unsafe_ones() {
        let rows = vec![
            vec![Cell::Link(
                "Upper".to_string(),
                "HTTPS://example.com/x".to_string(),
            )],
            vec![Cell::Link(
                "Hostile".to_string(),
                "JAVASCRIPT:alert(1)".to_string(),
            )],
        ];
        let html = data_table(None, &["Link"], &rows, None, None);
        assert!(
            html.contains("<a href=\"HTTPS://example.com/x\">Upper</a>"),
            "{html}"
        );
        assert!(!html.contains("href=\"JAVASCRIPT"), "{html}");
        assert!(html.contains("<td>Hostile</td>"), "{html}");
    }

    #[test]
    fn data_table_empty_state_replaces_the_body() {
        let empty = EmptyState {
            reason: "No findings in this snapshot. Five rule groups were checked where inputs were available.".to_string(),
            action: Some(("Open audit".to_string(), "/console/audit".to_string())),
        };
        let html = data_table(
            Some("Findings"),
            &["Finding", "Status"],
            &[],
            Some(empty),
            None,
        );
        assert!(html.contains("<table class=\"data-table\">"));
        assert!(!html.contains("<tbody"), "empty state replaces the body");
        assert!(html.contains("class=\"empty-state\""));
        assert!(html.contains("No findings in this snapshot."));
        assert!(html.contains(
            "<a class=\"action action-secondary\" href=\"/console/audit\">Open audit</a>"
        ));
        assert_eq!(html.matches("<th scope=\"col\">").count(), 2);
    }

    #[test]
    fn data_table_without_empty_state_keeps_an_honest_placeholder_row() {
        let html = data_table(None, &["A"], &[], None, None);
        assert!(html.contains("<tbody><tr><td colspan=\"1\" class=\"cell-muted\">Not available (no rows)</td></tr></tbody>"));
        assert!(!html.contains("<caption"));
    }

    #[test]
    fn data_table_escapes_hostile_cells() {
        let rows = vec![vec![
            Cell::Text(HOSTILE.to_string()),
            Cell::Mono(HOSTILE.to_string()),
            Cell::Num(HOSTILE.to_string()),
            Cell::Status(HOSTILE.to_string(), StatusKind::Failure),
            Cell::Link(HOSTILE.to_string(), "javascript:alert(1)".to_string()),
            Cell::Muted(HOSTILE.to_string()),
        ]];
        let html = data_table(Some(HOSTILE), &[HOSTILE, "b"], &rows, None, Some(HOSTILE));
        assert_escaped(&html);
        assert!(
            !html.contains("href=\"javascript:"),
            "javascript: link must not render"
        );
        assert!(!html.contains("<a href"), "{html}");
    }

    #[test]
    fn cell_link_renders_only_safe_destinations() {
        for safe in [
            "/console/audit",
            "https://example.com/x",
            "mailto:hello@apexmediation.ee",
            "pricing/",
            "#main",
        ] {
            let cell = cell_html(&Cell::Link("L".to_string(), safe.to_string()));
            assert!(cell.contains("<a href="), "{safe} should render a link");
        }
        for unsafe_ in [
            "javascript:alert(1)",
            "//evil.example",
            "data:text/html,x",
            "",
        ] {
            let cell = cell_html(&Cell::Link("L".to_string(), unsafe_.to_string()));
            assert!(!cell.contains("<a"), "{unsafe_} must not render a link");
        }
    }

    #[test]
    fn field_wires_label_help_error_and_aria() {
        let html = field(
            "platform",
            "Platform",
            Some("Choose the account that produced this report."),
            Some("Choose a supported platform."),
            &select(
                "platform",
                "platform",
                &[("applovin_max".to_string(), "AppLovin MAX".to_string())],
                None,
            ),
        );
        assert!(html.contains("<label class=\"field-label\" for=\"platform\">Platform</label>"));
        assert!(html.contains("id=\"platform-help\""));
        assert!(html.contains("id=\"platform-error\""));
        assert!(html.contains("aria-describedby=\"platform-help platform-error\""));
        assert!(html.contains("aria-invalid=\"true\""));
        assert!(html.contains("class=\"field field-invalid\""));
        assert!(html.contains("<select aria-describedby=\"platform-help platform-error\" aria-invalid=\"true\" class=\"input\" id=\"platform\" name=\"platform\""));
    }

    #[test]
    fn field_without_help_or_error_does_not_inject_aria() {
        let html = field(
            "q",
            "Search",
            None,
            None,
            &input("q", "q", "text", None, false, None),
        );
        assert!(!html.contains("aria-describedby"));
        assert!(!html.contains("aria-invalid"));
        assert!(html.contains("class=\"field\""));
    }

    #[test]
    fn field_escapes_everything_user_visible() {
        let html = field(
            HOSTILE,
            HOSTILE,
            Some(HOSTILE),
            Some(HOSTILE),
            &input("q", "q", "text", Some(HOSTILE), true, Some(HOSTILE)),
        );
        assert_escaped(&html);
        assert!(
            html.contains("value=\"&lt;script&gt;alert(&quot;x&quot;)&lt;/script&gt;&amp;&#39;\"")
        );
        assert!(html.contains(" required"));
    }

    #[test]
    fn controls_render_expected_attributes() {
        let text = input(
            "email",
            "work_email",
            "email",
            Some("you@studio.com"),
            true,
            Some("email"),
        );
        assert!(text.contains("type=\"email\""));
        assert!(text.contains("value=\"you@studio.com\""));
        assert!(text.contains(" autocomplete=\"email\""));
        assert!(text.contains(" required"));
        let area = textarea("notes", "notes", Some("hello"), 6, Some(2000));
        assert!(area.contains("rows=\"6\""));
        assert!(area.contains("maxlength=\"2000\""));
        assert!(area.contains(">hello</textarea>"));
        let empty_area = textarea("notes", "notes", None, 3, None);
        assert!(empty_area.contains("></textarea>"));
        let picker = select(
            "p",
            "platform",
            &[
                ("applovin_max".to_string(), "AppLovin MAX".to_string()),
                ("levelplay".to_string(), "LevelPlay".to_string()),
            ],
            Some("levelplay"),
        );
        assert!(picker.contains("<option value=\"levelplay\" selected>LevelPlay</option>"));
        assert!(picker.contains("<option value=\"applovin_max\">AppLovin MAX</option>"));
        let checked_box = checkbox("finalized", "finalized", "Finalized data only", true);
        assert!(checked_box.contains("<input type=\"checkbox\" id=\"finalized\" name=\"finalized\" checked> Finalized data only</label>"));
        let unchecked = checkbox("finalized", "finalized", "Finalized data only", false);
        assert!(!unchecked.contains("checked"));
    }

    #[test]
    fn form_error_summary_links_to_fields_and_escapes() {
        let html = form_error_summary(&[
            (
                "platform".to_string(),
                "Choose a supported platform.".to_string(),
            ),
            ("token".to_string(), HOSTILE.to_string()),
            ("skipped".to_string(), "  ".to_string()),
        ]);
        assert!(html.contains("role=\"alert\""));
        assert!(html.contains("<a href=\"#platform\">Choose a supported platform.</a>"));
        assert!(html.contains("<a href=\"#token\">&lt;script&gt;"));
        assert!(!html.contains("#skipped"));
        assert_escaped(&html);
        assert_eq!(form_error_summary(&[]), "");
        assert_eq!(form_error_summary(&[("a".to_string(), "".to_string())]), "");
    }

    #[test]
    fn details_renders_native_element() {
        let html = details("Technical details", "<p>json</p>", true);
        assert!(html.contains("<details class=\"details\" open>"));
        assert!(html.contains("<summary>Technical details</summary>"));
        assert!(html.contains("<div class=\"details-body\"><p>json</p></div>"));
        let closed = details(HOSTILE, "", false);
        assert!(!closed.contains(" open>"));
        assert_escaped(&closed);
    }

    #[test]
    fn filter_bar_is_a_get_form_with_labels_and_note() {
        let html = filter_bar(
            &[
                ("platform".to_string(), "applovin_max".to_string()),
                ("last_30_days".to_string(), "1".to_string()),
            ],
            "/console/audit",
            "5 findings / as of 14 Oct 2026, 08:10 UTC",
        );
        assert!(html.contains("<form class=\"filter-bar\" method=\"get\">"));
        assert!(
            html.contains("<label class=\"field-label\" for=\"filter-platform\">Platform</label>")
        );
        assert!(html.contains("name=\"platform\" value=\"applovin_max\""));
        assert!(html.contains(
            "<label class=\"field-label\" for=\"filter-last-30-days\">Last 30 days</label>"
        ));
        assert!(html.contains("type=\"submit\">Apply filters</button>"));
        assert!(
            html.contains("<a class=\"filter-clear\" href=\"/console/audit\">Clear filters</a>")
        );
        assert!(html.contains("<p class=\"table-note\">5 findings"));
        let hostile = filter_bar(
            &[("q".to_string(), HOSTILE.to_string())],
            "javascript:alert(1)",
            HOSTILE,
        );
        assert_escaped(&hostile);
        assert!(!hostile.contains("filter-clear"));
    }

    #[test]
    fn tabs_mark_the_active_tab() {
        let html = tabs(&[
            (
                "All states".to_string(),
                "/console/experiments".to_string(),
                true,
            ),
            (
                "Finalized data only".to_string(),
                "/console/experiments?final=1".to_string(),
                false,
            ),
        ]);
        assert!(html.contains("<nav class=\"tabs\" aria-label=\"Sections\">"));
        assert!(html.contains(
            "<a class=\"tab\" aria-current=\"page\" href=\"/console/experiments\">All states</a>"
        ));
        assert!(html.contains(
            "<a class=\"tab\" href=\"/console/experiments?final=1\">Finalized data only</a>"
        ));
    }

    #[test]
    fn pagination_links_carry_rel_and_note() {
        let html = pagination(
            Some("/console/ledger?page=1"),
            Some("/console/ledger?page=3"),
            "Page 2 of 4 / 24 records",
        );
        assert!(html.contains("<a class=\"pagination-link\" rel=\"prev\" href=\"/console/ledger?page=1\">Previous</a>"));
        assert!(html.contains("<span class=\"pagination-note\">Page 2 of 4 / 24 records</span>"));
        assert!(html.contains(
            "<a class=\"pagination-link\" rel=\"next\" href=\"/console/ledger?page=3\">Next</a>"
        ));
        let only_note = pagination(None, None, "24 records");
        assert!(!only_note.contains("rel=\"prev\""));
        assert!(!only_note.contains("rel=\"next\""));
    }

    #[test]
    fn breadcrumb_marks_the_last_part_current() {
        let html = breadcrumb(&["Workspace".to_string(), "Audit".to_string()]);
        assert!(html.contains("<nav class=\"breadcrumb\" aria-label=\"Breadcrumb\">"));
        assert!(html.contains("<li class=\"breadcrumb-item\">Workspace</li>"));
        assert!(html.contains("<li class=\"breadcrumb-item\" aria-current=\"page\">Audit</li>"));
        assert_eq!(breadcrumb(&[]), "");
    }

    #[test]
    fn actions_escape_and_guard_destinations() {
        let primary = action("View audit", "/console/audit", true);
        assert_eq!(
            primary,
            "<a class=\"action action-primary\" href=\"/console/audit\">View audit</a>"
        );
        let secondary = action(HOSTILE, "/x", false);
        assert!(secondary.contains("class=\"action action-secondary\""));
        assert_escaped(&secondary);
        let unsafe_ = action("Open", "javascript:alert(1)", true);
        assert!(!unsafe_.contains("<a "), "unsafe href renders inert");
        assert!(unsafe_.contains("Open</span>"));
        let button = action_button("Approve this test", "decision", "approve", true);
        assert_eq!(
            button,
            "<button class=\"action action-primary\" type=\"submit\" name=\"decision\" value=\"approve\">Approve this test</button>"
        );
        let hostile_button = action_button(HOSTILE, HOSTILE, HOSTILE, false);
        assert_escaped(&hostile_button);
    }

    #[test]
    fn csrf_hidden_is_a_hidden_input() {
        let html = csrf_hidden("tok\"en&");
        assert_eq!(
            html,
            "<input type=\"hidden\" name=\"csrf_token\" value=\"tok&quot;en&amp;\">"
        );
    }

    #[test]
    fn status_pair_shows_label_with_the_kind_text() {
        let html = status_pair("Read access", StatusKind::Verified);
        assert!(html.contains("<span class=\"status-pair-label\">Read access</span>"));
        assert!(html.contains("<span class=\"status status-verified\">Verified</span>"));
        let hostile = status_pair(HOSTILE, StatusKind::Attention);
        assert_escaped(&hostile);
        assert!(hostile.contains("Needs attention"));
    }

    #[test]
    fn injected_attrs_land_on_the_earliest_control_tag_of_a_fragment() {
        // Two candidate controls: the earliest one takes the label wiring,
        // the later one stays untouched, and the fragment keeps its order.
        let html = inject_control_attrs(
            "<textarea class=\"input\"></textarea><input class=\"input\">",
            "f1-help",
            true,
        );
        assert!(
            html.starts_with("<textarea aria-describedby=\"f1-help\" aria-invalid=\"true\""),
            "{html}"
        );
        assert_eq!(html.matches("aria-describedby").count(), 1, "{html}");
        // A tag-like prefix is not a control tag start and gets nothing.
        assert_eq!(
            inject_control_attrs("<inputx>", "f1-help", false),
            "<inputx>"
        );
        // Without described-by or invalid, the fragment is returned untouched.
        assert_eq!(inject_control_attrs("<input>", "", false), "<input>");
    }
}
