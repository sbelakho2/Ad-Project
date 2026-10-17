//! Server-rendered SVG charts: no JavaScript, no hover-only meaning.
//!
//! Every chart carries an SVG `<title>` and `<desc>`, a legend with text
//! labels and an adjacent data table, so the same numbers are reachable
//! without color vision or a pointer. Missing points are gaps, never zeros:
//! a `None` (or non-finite) point breaks the line, draws no bar and renders
//! as `Not available (no data)` in the table.
//!
//! Colors follow the data grammar: control is gray ([`ChartColor::Control`]),
//! test is cobalt ([`ChartColor::Test`]), ink is reserved for neutral series.

use std::fmt::Write as _;

use crate::escape::html_escape;
use crate::format::or_not_available;
use crate::tokens;

const PAD_LEFT: f64 = 44.0;
const PAD_RIGHT: f64 = 12.0;
const PAD_TOP: f64 = 16.0;
const PAD_BOTTOM: f64 = 28.0;

/// A series color from the data grammar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChartColor {
    /// The control series (gray).
    Control,
    /// The test series (cobalt).
    Test,
    /// A neutral series (ink).
    Ink,
}

impl ChartColor {
    /// The frozen hex for this series color.
    pub(crate) fn hex(&self) -> &'static str {
        match self {
            ChartColor::Control => tokens::SECONDARY,
            ChartColor::Test => tokens::COBALT,
            ChartColor::Ink => tokens::INK,
        }
    }

    /// The stylesheet class carrying the series color as a background for
    /// the legend swatch (`background:<the exact hex above>`), so the legend
    /// stays free of inline styles.
    pub(crate) fn swatch_class(&self) -> &'static str {
        match self {
            ChartColor::Control => "chart-swatch-control",
            ChartColor::Test => "chart-swatch-test",
            ChartColor::Ink => "chart-swatch-ink",
        }
    }
}

/// One chart series: a label, a color and one optional value per x label.
#[derive(Debug, Clone, PartialEq)]
pub struct ChartSeries {
    /// The series label shown in the legend, direct label and table.
    pub label: String,
    /// The series color.
    pub color: ChartColor,
    /// The values; `None` is a missing point and renders as a gap.
    pub points: Vec<Option<f64>>,
}

/// The shared chart frame: title, description, size, y-axis label, x labels
/// and the zero-reference flag.
#[derive(Debug, Clone, PartialEq)]
pub struct ChartSpec {
    /// The chart title (SVG title, legend heading and table caption).
    pub title: String,
    /// The chart description for assistive technology.
    pub desc: String,
    /// Width in CSS pixels.
    pub width: u32,
    /// Height in CSS pixels.
    pub height: u32,
    /// The y-axis label.
    pub y_label: String,
    /// One label per x position.
    pub x_labels: Vec<String>,
    /// Draw an explicit zero reference line when zero is inside the range.
    pub zero_line: bool,
}

/// A line chart with one polyline per contiguous run of present points,
/// point markers per series color, optional zero reference and an adjacent
/// data table.
pub fn line_chart(spec: &ChartSpec, series: &[ChartSeries], y_min: f64, y_max: f64) -> String {
    let Some(box_) = plot_box(spec) else {
        return unavailable(spec, "chart size is too small");
    };
    if !y_min.is_finite() || !y_max.is_finite() || y_max <= y_min {
        return unavailable(spec, "chart range is invalid");
    }
    if series.iter().any(|s| s.points.len() > spec.x_labels.len()) {
        return unavailable(spec, "series exceed the x labels");
    }
    let (x0, inner_w, y0, inner_h) = box_;
    let bottom = y0 + inner_h;
    let count = spec.x_labels.len().max(1);

    let mut svg = String::new();
    let _ = write!(
        svg,
        "<svg class=\"chart-svg\" viewBox=\"0 0 {} {}\" width=\"{}\" height=\"{}\" role=\"img\">",
        spec.width, spec.height, spec.width, spec.height
    );
    let _ = write!(
        svg,
        "<title>{}</title><desc>{}</desc>",
        html_escape(&spec.title),
        html_escape(&spec.desc)
    );
    let _ = write!(
        svg,
        "<text class=\"chart-text\" x=\"0\" y=\"12\" font-size=\"12\">{}</text>",
        html_escape(&spec.y_label)
    );
    let _ = write!(
        svg,
        "<line class=\"chart-baseline\" x1=\"{}\" y1=\"{}\" x2=\"{}\" y2=\"{}\"/>",
        coord(x0),
        coord(bottom),
        coord(x0 + inner_w),
        coord(bottom)
    );
    let _ = write!(
        svg,
        "<text class=\"chart-text\" x=\"{}\" y=\"{}\" text-anchor=\"end\" font-size=\"12\">{}</text>",
        coord(x0 - 8.0),
        coord(bottom + 4.0),
        fmt_axis(y_min)
    );
    let _ = write!(
        svg,
        "<text class=\"chart-text\" x=\"{}\" y=\"{}\" text-anchor=\"end\" font-size=\"12\">{}</text>",
        coord(x0 - 8.0),
        coord(y0 + 4.0),
        fmt_axis(y_max)
    );
    if spec.zero_line && y_min < 0.0 && y_max > 0.0 {
        let zero = y_pos(0.0, y_min, y_max, y0, inner_h);
        let _ = write!(
            svg,
            "<line class=\"chart-zero\" x1=\"{}\" y1=\"{}\" x2=\"{}\" y2=\"{}\"/>",
            coord(x0),
            coord(zero),
            coord(x0 + inner_w),
            coord(zero)
        );
    }
    for (index, label) in x_label_positions(&spec.x_labels) {
        let x = x_pos(index, count, x0, inner_w);
        let _ = write!(
            svg,
            "<text class=\"chart-text\" x=\"{}\" y=\"{}\" text-anchor=\"middle\" font-size=\"12\">{}</text>",
            coord(x),
            coord(bottom + 20.0),
            html_escape(label)
        );
    }

    for series in series {
        let color = series.color.hex();
        let mut segments: Vec<Vec<(f64, f64)>> = Vec::new();
        let mut current: Vec<(f64, f64)> = Vec::new();
        for (index, point) in series.points.iter().enumerate() {
            match point {
                Some(value) if value.is_finite() => {
                    let x = x_pos(index, count, x0, inner_w);
                    let y = y_pos(*value, y_min, y_max, y0, inner_h);
                    current.push((x, y));
                }
                _ => {
                    if !current.is_empty() {
                        segments.push(std::mem::take(&mut current));
                    }
                }
            }
        }
        if !current.is_empty() {
            segments.push(current);
        }
        let mut points: Vec<(f64, f64)> = Vec::new();
        for segment in &segments {
            points.extend(segment.iter().copied());
            if segment.len() >= 2 {
                let line: Vec<String> = segment
                    .iter()
                    .map(|(x, y)| format!("{},{}", coord(*x), coord(*y)))
                    .collect();
                let _ = write!(
                    svg,
                    "<polyline points=\"{}\" fill=\"none\" stroke=\"{color}\" stroke-width=\"2\"/>",
                    line.join(" ")
                );
            }
        }
        for (x, y) in &points {
            let _ = write!(svg, "{}", marker(*x, *y, series.color));
        }
        if let Some((x, y)) = points.last().copied() {
            let anchor = if x > f64::from(spec.width) * 0.6 {
                "end"
            } else {
                "start"
            };
            let dx = if anchor == "end" { -4.0 } else { 4.0 };
            let ty = (y - 6.0).max(12.0);
            let _ = write!(
                svg,
                "<text class=\"chart-text\" x=\"{}\" y=\"{}\" text-anchor=\"{anchor}\" font-size=\"12\">{}</text>",
                coord(x + dx),
                coord(ty),
                html_escape(&series.label)
            );
        }
    }
    svg.push_str("</svg>");

    let headers: Vec<String> = std::iter::once("Series".to_string())
        .chain(spec.x_labels.iter().cloned())
        .collect();
    let rows: Vec<(String, Vec<Option<f64>>)> = series
        .iter()
        .map(|s| (s.label.clone(), s.points.clone()))
        .collect();

    let mut out = String::from("<figure class=\"chart\">");
    out.push_str(&svg);
    out.push_str(&legend(series));
    out.push_str(&chart_table(&spec.title, &headers, &rows));
    out.push_str("</figure>");
    out
}

/// A bar chart over one series. Zero stays on the axis: bars grow from the
/// zero baseline (negative values below it) and a zero line is always
/// drawn. Missing points draw no bar and read `Not available (no data)` in
/// the table.
pub fn bar_chart(spec: &ChartSpec, values: &[Option<f64>], color: ChartColor) -> String {
    let Some((x0, inner_w, y0, inner_h)) = plot_box(spec) else {
        return unavailable(spec, "chart size is too small");
    };
    if values.len() > spec.x_labels.len() {
        return unavailable(spec, "bars exceed the x labels");
    }
    let count = spec.x_labels.len().max(1);
    let mut low = 0.0f64;
    let mut high = 0.0f64;
    let mut any = false;
    for value in values.iter().flatten() {
        if value.is_finite() {
            low = low.min(*value);
            high = high.max(*value);
            any = true;
        }
    }
    if !any {
        high = 1.0;
    }
    if high == low {
        high = low + 1.0;
    }
    let bottom = y0 + inner_h;
    let zero_y = y_pos(0.0, low, high, y0, inner_h);
    let bar_width = (inner_w / count as f64 * 0.6).max(1.0);

    let mut svg = String::new();
    let _ = write!(
        svg,
        "<svg class=\"chart-svg\" viewBox=\"0 0 {} {}\" width=\"{}\" height=\"{}\" role=\"img\">",
        spec.width, spec.height, spec.width, spec.height
    );
    let _ = write!(
        svg,
        "<title>{}</title><desc>{}</desc>",
        html_escape(&spec.title),
        html_escape(&spec.desc)
    );
    let _ = write!(
        svg,
        "<text class=\"chart-text\" x=\"0\" y=\"12\" font-size=\"12\">{}</text>",
        html_escape(&spec.y_label)
    );
    let _ = write!(
        svg,
        "<line class=\"chart-zero\" x1=\"{}\" y1=\"{}\" x2=\"{}\" y2=\"{}\"/>",
        coord(x0),
        coord(zero_y),
        coord(x0 + inner_w),
        coord(zero_y)
    );
    let _ = write!(
        svg,
        "<line class=\"chart-baseline\" x1=\"{}\" y1=\"{}\" x2=\"{}\" y2=\"{}\"/>",
        coord(x0),
        coord(bottom),
        coord(x0 + inner_w),
        coord(bottom)
    );
    let _ = write!(
        svg,
        "<text class=\"chart-text\" x=\"{}\" y=\"{}\" text-anchor=\"end\" font-size=\"12\">{}</text>",
        coord(x0 - 8.0),
        coord(zero_y + 4.0),
        fmt_axis(0.0)
    );
    let _ = write!(
        svg,
        "<text class=\"chart-text\" x=\"{}\" y=\"{}\" text-anchor=\"end\" font-size=\"12\">{}</text>",
        coord(x0 - 8.0),
        coord(y_pos(high, low, high, y0, inner_h) + 4.0),
        fmt_axis(high)
    );
    for (index, label) in x_label_positions(&spec.x_labels) {
        let x = x_pos(index, count, x0, inner_w);
        let _ = write!(
            svg,
            "<text class=\"chart-text\" x=\"{}\" y=\"{}\" text-anchor=\"middle\" font-size=\"12\">{}</text>",
            coord(x),
            coord(bottom + 20.0),
            html_escape(label)
        );
    }
    let color_hex = color.hex();
    for (index, value) in values.iter().enumerate() {
        let Some(value) = value else { continue };
        if !value.is_finite() {
            continue;
        }
        let x = x_pos(index, count, x0, inner_w);
        let value_y = y_pos(*value, low, high, y0, inner_h);
        let (top, height) = if value_y <= zero_y {
            (value_y, zero_y - value_y)
        } else {
            (zero_y, value_y - zero_y)
        };
        let _ = write!(
            svg,
            "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" fill=\"{color_hex}\"/>",
            coord(x - bar_width / 2.0),
            coord(top),
            coord(bar_width),
            coord(height.max(1.0))
        );
    }
    svg.push_str("</svg>");

    let headers: Vec<String> = std::iter::once("Value".to_string())
        .chain(spec.x_labels.iter().cloned())
        .collect();
    let rows = vec![("Bar".to_string(), values.to_vec())];

    let mut out = String::from("<figure class=\"chart\">");
    out.push_str(&svg);
    out.push_str(&chart_table(&spec.title, &headers, &rows));
    out.push_str("</figure>");
    out
}

/// A compact inline sparkline. Gaps break the line; the description states
/// the point and gap counts so the shape is not the only information.
pub fn sparkline(values: &[Option<f64>]) -> String {
    let finite: Vec<(usize, f64)> = values
        .iter()
        .enumerate()
        .filter_map(|(index, value)| value.filter(|v| v.is_finite()).map(|v| (index, v)))
        .collect();
    let gaps = values.len() - finite.len();
    let count = values.len().max(1);
    let (min, max) = finite
        .iter()
        .fold((f64::INFINITY, f64::NEG_INFINITY), |acc, (_, v)| {
            (acc.0.min(*v), acc.1.max(*v))
        });
    let position = |index: usize, value: f64| -> (f64, f64) {
        let x = if count <= 1 {
            60.0
        } else {
            index as f64 * 120.0 / (count - 1) as f64
        };
        let y = if max <= min {
            16.0
        } else {
            26.0 - (value - min) / (max - min) * 20.0
        };
        (x, y)
    };
    let mut svg = String::from(
        "<svg class=\"sparkline\" viewBox=\"0 0 120 32\" width=\"120\" height=\"32\" role=\"img\">",
    );
    let _ = write!(
        svg,
        "<title>Sparkline</title><desc>{} points, {} missing</desc>",
        finite.len(),
        gaps
    );
    let mut segments: Vec<Vec<(f64, f64)>> = Vec::new();
    let mut current: Vec<(f64, f64)> = Vec::new();
    for (index, value) in values.iter().enumerate() {
        match value {
            Some(value) if value.is_finite() => current.push(position(index, *value)),
            _ => {
                if !current.is_empty() {
                    segments.push(std::mem::take(&mut current));
                }
            }
        }
    }
    if !current.is_empty() {
        segments.push(current);
    }
    for segment in &segments {
        if segment.len() >= 2 {
            let line: Vec<String> = segment
                .iter()
                .map(|(x, y)| format!("{},{}", coord(*x), coord(*y)))
                .collect();
            let _ = write!(
                svg,
                "<polyline class=\"sparkline-line\" points=\"{}\" stroke=\"{}\"/>",
                line.join(" "),
                tokens::COBALT
            );
        }
    }
    svg.push_str("</svg>");
    svg
}

/// A legend with text labels so color is never the only channel.
fn legend(series: &[ChartSeries]) -> String {
    if series.is_empty() {
        return String::new();
    }
    let mut out = String::from("<ul class=\"chart-legend\">");
    for series in series {
        let _ = write!(
            out,
            "<li class=\"chart-legend-item\"><span class=\"chart-legend-swatch {}\"></span>{}</li>",
            series.color.swatch_class(),
            html_escape(&series.label)
        );
    }
    out.push_str("</ul>");
    out
}

/// The adjacent data table: the same numbers, with missing points stated as
/// `Not available (no data)`.
fn chart_table(title: &str, headers: &[String], rows: &[(String, Vec<Option<f64>>)]) -> String {
    let mut out = String::from("<table class=\"data-table chart-table\">");
    let _ = write!(
        out,
        "<caption class=\"table-caption\">{}</caption><thead><tr>",
        html_escape(title)
    );
    for header in headers {
        let _ = write!(out, "<th scope=\"col\">{}</th>", html_escape(header));
    }
    out.push_str("</tr></thead><tbody>");
    for (label, values) in rows {
        let _ = write!(out, "<tr><th scope=\"row\">{}</th>", html_escape(label));
        for value in values {
            match value {
                Some(value) if value.is_finite() => {
                    let _ = write!(
                        out,
                        "<td class=\"cell-num\">{}</td>",
                        html_escape(&fmt_point(*value))
                    );
                }
                _ => {
                    let _ = write!(
                        out,
                        "<td class=\"cell-muted\">{}</td>",
                        html_escape(&or_not_available("no data"))
                    );
                }
            }
        }
        out.push_str("</tr>");
    }
    out.push_str("</tbody></table>");
    out
}

/// The plot frame `(x0, inner_width, y0, inner_height)`, or `None` when the
/// requested size cannot hold the axes.
fn plot_box(spec: &ChartSpec) -> Option<(f64, f64, f64, f64)> {
    let width = f64::from(spec.width);
    let height = f64::from(spec.height);
    if width < PAD_LEFT + PAD_RIGHT + 40.0 || height < PAD_TOP + PAD_BOTTOM + 40.0 {
        return None;
    }
    Some((
        PAD_LEFT,
        width - PAD_LEFT - PAD_RIGHT,
        PAD_TOP,
        height - PAD_TOP - PAD_BOTTOM,
    ))
}

fn x_pos(index: usize, count: usize, x0: f64, inner_w: f64) -> f64 {
    if count <= 1 {
        x0 + inner_w / 2.0
    } else {
        x0 + inner_w * index as f64 / (count - 1) as f64
    }
}

/// The y position of `value`, clamped into the plot range so geometry stays
/// inside the frame (the table carries the exact numbers).
fn y_pos(value: f64, y_min: f64, y_max: f64, y0: f64, inner_h: f64) -> f64 {
    let clamped = value.clamp(y_min, y_max);
    y0 + inner_h * (y_max - clamped) / (y_max - y_min)
}

/// The marker shape per series color: circles for control and ink, diamonds
/// for test, so the series stay distinct without color.
fn marker(x: f64, y: f64, color: ChartColor) -> String {
    match color {
        ChartColor::Test => format!(
            "<polygon points=\"{},{} {},{} {},{} {},{}\" fill=\"{}\"/>",
            coord(x),
            coord(y - 3.0),
            coord(x + 3.0),
            coord(y),
            coord(x),
            coord(y + 3.0),
            coord(x - 3.0),
            coord(y),
            color.hex()
        ),
        _ => format!(
            "<circle cx=\"{}\" cy=\"{}\" r=\"2.5\" fill=\"{}\"/>",
            coord(x),
            coord(y),
            color.hex()
        ),
    }
}

/// Which x labels to draw: all when at most twelve, otherwise an evenly
/// spaced subset that always includes the first and last label.
fn x_label_positions(labels: &[String]) -> Vec<(usize, &String)> {
    let count = labels.len();
    if count == 0 {
        return Vec::new();
    }
    let step = count.div_ceil(12);
    labels
        .iter()
        .enumerate()
        .filter(|(index, _)| index % step == 0 || *index == count - 1)
        .collect()
}

fn coord(value: f64) -> String {
    format!("{value:.1}")
}

/// Axis tick text: integers stay integers, fractional values keep one
/// decimal.
fn fmt_axis(value: f64) -> String {
    if value == 0.0 {
        "0".to_string()
    } else if value.fract() == 0.0 {
        format!("{value:.0}")
    } else {
        format!("{value:.1}")
    }
}

/// Table cell text for a present point.
fn fmt_point(value: f64) -> String {
    if value.fract() == 0.0 {
        format!("{value:.0}")
    } else {
        format!("{value:.2}")
    }
}

/// The honest fallback when a chart cannot be drawn: the title and the
/// reason, in app phrasing, with no fabricated geometry.
fn unavailable(spec: &ChartSpec, reason: &str) -> String {
    format!(
        "<figure class=\"chart\"><p class=\"chart-unavailable\">{}: {}</p></figure>",
        html_escape(&spec.title),
        html_escape(&or_not_available(reason))
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(title: &str, x_labels: &[&str]) -> ChartSpec {
        ChartSpec {
            title: title.to_string(),
            desc: "Observed lift and uncertainty".to_string(),
            width: 640,
            height: 240,
            y_label: "Revenue".to_string(),
            x_labels: x_labels.iter().map(|s| s.to_string()).collect(),
            zero_line: true,
        }
    }

    fn series(label: &str, color: ChartColor, points: Vec<Option<f64>>) -> ChartSeries {
        ChartSeries {
            label: label.to_string(),
            color,
            points,
        }
    }

    #[test]
    fn colors_match_the_data_grammar() {
        assert_eq!(ChartColor::Control.hex(), tokens::SECONDARY);
        assert_eq!(ChartColor::Test.hex(), tokens::COBALT);
        assert_eq!(ChartColor::Ink.hex(), tokens::INK);
    }

    /// One bar mark per present point and one table row per x label at
    /// 10_000 points: the render is linear in the input, and a point is
    /// never drawn or tabulated twice.
    #[test]
    fn bar_chart_draws_one_mark_per_point_at_ten_thousand_points() {
        const N: usize = 10_000;
        let spec = ChartSpec {
            title: "Sends".into(),
            desc: "per day".into(),
            width: 720,
            height: 220,
            y_label: "Sends".into(),
            x_labels: (0..N)
                .map(|i| format!("2026-01-{:02}", i % 28 + 1))
                .collect(),
            zero_line: false,
        };
        let values: Vec<Option<f64>> = (0..N)
            .map(|i| {
                if i % 100 == 7 {
                    None
                } else {
                    Some((i % 97) as f64)
                }
            })
            .collect();
        let present = values.iter().filter(|v| v.is_some()).count();
        assert_eq!(present, 9_900);
        let html = bar_chart(&spec, &values, ChartColor::Ink);
        assert_eq!(
            html.matches("<rect ").count(),
            present,
            "one bar per present point"
        );
        // The table is transposed (one row per series, one column per x
        // label): the header row plus one bar row, and exactly one cell per
        // x label after the leading value cell.
        assert_eq!(html.matches("<tr").count(), 2, "header plus the bar row");
        assert_eq!(
            html.matches("<td").count(),
            N,
            "one cell per x label (the leading value cell is a row header)"
        );
    }

    #[test]
    fn legend_swatches_use_stylesheet_classes_not_inline_styles() {
        let html = line_chart(
            &spec("Lift", &["a", "b"]),
            &[
                series("Control", ChartColor::Control, vec![Some(1.0), Some(2.0)]),
                series("Test", ChartColor::Test, vec![Some(1.0), Some(2.0)]),
            ],
            0.0,
            2.0,
        );
        assert!(html.contains("class=\"chart-legend-swatch chart-swatch-control\""));
        assert!(html.contains("class=\"chart-legend-swatch chart-swatch-test\""));
        assert!(
            !html.contains("style="),
            "the legend must not inline styles"
        );
        assert_eq!(ChartColor::Ink.swatch_class(), "chart-swatch-ink");
        let css = tokens::console_css();
        for class in [
            "chart-swatch-control",
            "chart-swatch-test",
            "chart-swatch-ink",
        ] {
            assert!(
                css.contains(&format!(".{class}")),
                "stylesheet carries no rule for .{class}"
            );
        }
    }

    #[test]
    fn line_chart_has_title_desc_legend_and_table() {
        let html = line_chart(
            &spec("Observed lift", &["01 Oct", "02 Oct", "03 Oct"]),
            &[
                series(
                    "Control",
                    ChartColor::Control,
                    vec![Some(1.0), Some(1.1), Some(1.2)],
                ),
                series(
                    "Test",
                    ChartColor::Test,
                    vec![Some(1.0), Some(1.4), Some(1.5)],
                ),
            ],
            0.0,
            2.0,
        );
        assert!(html.contains("<title>Observed lift</title>"));
        assert!(html.contains("<desc>Observed lift and uncertainty</desc>"));
        assert!(html.contains("class=\"chart-legend\""));
        assert!(html.contains(">Control</li>"));
        assert!(html.contains(">Test</li>"));
        assert!(html.contains("<table class=\"data-table chart-table\">"));
        assert!(html.contains("<caption class=\"table-caption\">Observed lift</caption>"));
        assert!(html.contains("<th scope=\"col\">Series</th>"));
        assert!(html.contains("<th scope=\"row\">Test</th>"));
        assert!(html.contains("<td class=\"cell-num\">1.40</td>"));
        assert!(html.contains("stroke=\"#536568\""));
        assert!(html.contains("stroke=\"#2849d8\""));
        assert!(!html.contains("<script"));
    }

    #[test]
    fn line_chart_missing_points_are_gaps_never_zeros() {
        // A gap between x0 and x2 must not draw a line from x0 to x2 and must
        // not read as 0 in the table.
        let html = line_chart(
            &spec("Trend", &["a", "b", "c"]),
            &[series(
                "Test",
                ChartColor::Test,
                vec![Some(1.0), None, Some(3.0)],
            )],
            0.0,
            4.0,
        );
        assert_eq!(
            html.matches("<polyline").count(),
            0,
            "single points do not connect"
        );
        assert_eq!(
            html.matches("<polygon").count(),
            2,
            "both present points drawn"
        );
        assert!(html.contains("Not available (no data)"));
        assert!(
            !html.contains(">0</td>") && !html.contains(">0.00</td>"),
            "a missing point must not be rendered as zero: {html}"
        );
    }

    #[test]
    fn line_chart_gap_breaks_a_longer_run() {
        let html = line_chart(
            &spec("Trend", &["a", "b", "c", "d"]),
            &[series(
                "Test",
                ChartColor::Test,
                vec![Some(1.0), None, Some(3.0), Some(4.0)],
            )],
            0.0,
            4.0,
        );
        assert_eq!(html.matches("<polyline").count(), 1, "only c-d connect");
        assert_eq!(html.matches("<polygon").count(), 3);
    }

    #[test]
    fn line_chart_draws_zero_reference_when_the_range_crosses_zero() {
        let html = line_chart(
            &spec("Lift", &["a", "b"]),
            &[series(
                "Test",
                ChartColor::Test,
                vec![Some(-1.0), Some(1.0)],
            )],
            -2.0,
            2.0,
        );
        assert!(html.contains("class=\"chart-zero\""));
        let no_zero = line_chart(
            &spec("Lift", &["a", "b"]),
            &[series("Test", ChartColor::Test, vec![Some(1.0), Some(2.0)])],
            0.0,
            2.0,
        );
        assert!(!no_zero.contains("class=\"chart-zero\""));
    }

    #[test]
    fn charts_draw_safe_axes_direct_labels_and_escaped_text() {
        let mut s = spec("Lift", &["a", "b", "c"]);
        s.y_label = "<script>".to_string();
        let html = line_chart(
            &s,
            &[series(
                "<script>\"x",
                ChartColor::Test,
                vec![Some(1.0), Some(2.0), Some(3.0)],
            )],
            0.0,
            3.0,
        );
        assert!(!html.contains("<script"));
        assert!(html.contains("&lt;script&gt;"));
        assert!(html.contains("&lt;script&gt;&quot;x</text>"));
        assert!(html.contains("class=\"chart-text\""));
    }

    #[test]
    fn invalid_ranges_and_sizes_return_the_honest_fallback() {
        let s = spec("Lift", &["a", "b"]);
        let bad_range = line_chart(&s, &[], 5.0, 5.0);
        assert!(bad_range.contains("chart-unavailable"));
        assert!(bad_range.contains("Not available (chart range is invalid)"));
        let mut small = spec("Lift", &["a", "b"]);
        small.height = 40;
        let tiny = line_chart(&small, &[], 0.0, 1.0);
        assert!(tiny.contains("Not available (chart size is too small)"));
        let too_many = line_chart(
            &s,
            &[series(
                "T",
                ChartColor::Test,
                vec![Some(1.0), Some(2.0), Some(3.0)],
            )],
            0.0,
            3.0,
        );
        assert!(too_many.contains("Not available (series exceed the x labels)"));
        assert!(line_chart(&s, &[], f64::NAN, 1.0).contains("chart range is invalid"));
    }

    #[test]
    fn bar_chart_keeps_zero_on_the_axis_and_gaps_stay_gaps() {
        let html = bar_chart(
            &spec("Recovered", &["DE", "US", "FR"]),
            &[Some(4.0), None, Some(-2.0)],
            ChartColor::Test,
        );
        assert!(html.contains("<title>Recovered</title>"));
        assert!(html.contains("class=\"chart-zero\""));
        assert_eq!(
            html.matches("<rect").count(),
            2,
            "the missing bar draws nothing"
        );
        assert!(html.contains("Not available (no data)"));
        assert!(html.contains("<td class=\"cell-num\">4</td>"));
        assert!(html.contains("<td class=\"cell-num\">-2</td>"));
        assert!(!html.contains("<polyline"));
    }

    #[test]
    fn bar_chart_with_no_finite_values_still_renders_honestly() {
        let html = bar_chart(
            &spec("Recovered", &["DE", "US"]),
            &[None, None],
            ChartColor::Control,
        );
        assert!(!html.contains("<rect"));
        assert_eq!(html.matches("Not available (no data)").count(), 2);
        assert!(html.contains("<title>Recovered</title>"));
    }

    #[test]
    fn sparkline_breaks_at_gaps_and_describes_counts() {
        let html = sparkline(&[Some(1.0), None, Some(3.0), Some(4.0)]);
        assert!(html.contains("<title>Sparkline</title>"));
        assert!(html.contains("<desc>3 points, 1 missing</desc>"));
        assert_eq!(html.matches("<polyline").count(), 1);
        assert!(html.contains("class=\"sparkline-line\""));
        let flat = sparkline(&[Some(2.0), Some(2.0)]);
        assert_eq!(flat.matches("<polyline").count(), 1);
        let empty = sparkline(&[]);
        assert!(empty.contains("<desc>0 points, 0 missing</desc>"));
        assert!(!empty.contains("<polyline"));
    }

    #[test]
    fn chart_table_headers_exist_for_every_column_of_every_series() {
        let html = bar_chart(
            &spec("X", &["a", "b"]),
            &[Some(1.0), Some(2.0)],
            ChartColor::Control,
        );
        assert_eq!(html.matches("<th ").count(), 4);
        assert!(html.contains("<th scope=\"col\">Value</th>"));
    }

    #[test]
    fn a_last_point_on_the_left_half_anchors_start_and_empty_series_skip_labels() {
        // The only present point sits far left of 60% of the width, so the
        // direct label anchors at "start"; the all-missing series draws no
        // direct label at all.
        let html = line_chart(
            &spec("Lift", &["a", "b"]),
            &[
                series("Control", ChartColor::Control, vec![Some(1.0), None]),
                series("Test", ChartColor::Test, vec![None, None]),
            ],
            0.0,
            2.0,
        );
        assert!(html.contains("text-anchor=\"start\""), "{html}");
        assert!(html.contains(">Control</text>"), "{html}");
        assert!(!html.contains(">Test</text>"), "no points, no direct label");
        assert_eq!(html.matches("<circle").count(), 1);
    }

    #[test]
    fn fractional_axes_and_single_labels_use_the_helper_paths() {
        // count == 1 centers the single x position; a fractional y axis
        // renders with one decimal.
        let single = line_chart(
            &spec("Lift", &["a"]),
            &[series("Test", ChartColor::Test, vec![Some(0.5)])],
            0.3,
            1.7,
        );
        assert!(single.contains(">0.3</text>"), "{single}");
        assert!(single.contains(">1.7</text>"), "{single}");
        assert!(
            single.contains("text-anchor=\"middle\" font-size=\"12\">a</text>"),
            "{single}"
        );
        let empty = line_chart(&spec("Lift", &[]), &[], 0.0, 1.0);
        assert!(!empty.contains("chart-legend"), "{empty}");
        assert!(!empty.contains("text-anchor=\"middle\""), "{empty}");
    }

    #[test]
    fn bars_refuse_bad_frames_bad_counts_and_stay_flat_on_zero() {
        let mut small = spec("Lift", &["a"]);
        small.width = 60;
        assert!(bar_chart(&small, &[Some(1.0)], ChartColor::Ink)
            .contains("Not available (chart size is too small)"));
        let too_many = bar_chart(
            &spec("Lift", &["a"]),
            &[Some(1.0), Some(2.0)],
            ChartColor::Ink,
        );
        assert!(too_many.contains("Not available (bars exceed the x labels)"));
        // A flat zero series keeps a drawable range and skips NaNs.
        let flat = bar_chart(
            &spec("Lift", &["a", "b"]),
            &[Some(0.0), Some(f64::NAN)],
            ChartColor::Ink,
        );
        assert!(flat.contains("<rect"), "the zero bar still draws: {flat}");
        assert_eq!(flat.matches("<rect").count(), 1);
        assert!(flat.contains("Not available (no data)"), "{flat}");
    }

    #[test]
    fn a_one_point_sparkline_still_renders_its_single_position() {
        let single = sparkline(&[Some(2.0)]);
        assert!(
            single.contains("<desc>1 points, 0 missing</desc>"),
            "{single}"
        );
        assert!(!single.contains("<polyline"), "one point cannot connect");
    }
}
