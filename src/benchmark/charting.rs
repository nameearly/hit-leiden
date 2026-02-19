// Copyright 2026 naadir jeewa
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.
//
// SPDX-License-Identifier: Apache-2.0

use crate::core::types::BenchmarkRun;
use plotly::common::{AxisSide, Mode};
use plotly::layout::{Axis, AxisType, Layout, Legend};
use plotly::{Plot, Scatter};
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::path::PathBuf;

/// Generate a self-contained interactive HTML chart from benchmark results.
/// Creates two vertically stacked subplots using explicit axis domains:
/// 1. Top: HIT-Leiden vs igraph batch times + speedup (secondary y-axis)
/// 2. Bottom: Modularity comparison (HIT-Leiden vs igraph)
pub fn generate_chart(
    run: &BenchmarkRun,
    output_path: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    if let Some(parent) = output_path.parent() {
        fs::create_dir_all(parent)?;
    }

    let has_igraph = run.batches.iter().any(|b| b.igraph_time_ms > 0.0);

    let batch_indices: Vec<usize> = run.batches.iter().map(|b| b.batch_idx + 1).collect();
    let hit_times: Vec<f64> = run.batches.iter().map(|b| b.hit_leiden_time_ms).collect();
    let hit_modularity: Vec<f64> = run.batches.iter().map(|b| b.modularity).collect();

    let mut plot = Plot::new();

    // Top subplot: Timing comparison (xaxis/yaxis, yaxis2 overlay)
    let hit_trace = Scatter::new(batch_indices.clone(), hit_times)
        .mode(Mode::LinesMarkers)
        .name("HIT-Leiden time");
    plot.add_trace(hit_trace);

    if has_igraph {
        let igraph_times: Vec<f64> = run.batches.iter().map(|b| b.igraph_time_ms).collect();
        let igraph_trace = Scatter::new(batch_indices.clone(), igraph_times)
            .mode(Mode::LinesMarkers)
            .name("igraph time");
        plot.add_trace(igraph_trace);

        let speedups: Vec<f64> = run.batches.iter().map(|b| b.speedup).collect();
        let speedup_trace = Scatter::new(batch_indices.clone(), speedups)
            .mode(Mode::LinesMarkers)
            .name("Speedup")
            .y_axis("y2");
        plot.add_trace(speedup_trace);
    }

    // Bottom subplot: Modularity comparison (xaxis3/yaxis3)
    let hit_mod_trace = Scatter::new(batch_indices.clone(), hit_modularity)
        .mode(Mode::LinesMarkers)
        .name("HIT-Leiden modularity")
        .x_axis("x2")
        .y_axis("y3");
    plot.add_trace(hit_mod_trace);

    if has_igraph {
        let igraph_modularity: Vec<f64> = run.batches.iter().map(|b| b.igraph_modularity).collect();
        let igraph_mod_trace = Scatter::new(batch_indices, igraph_modularity)
            .mode(Mode::LinesMarkers)
            .name("igraph modularity")
            .x_axis("x2")
            .y_axis("y3");
        plot.add_trace(igraph_mod_trace);
    }

    let truncated_note = if run.truncated {
        " (truncated by timeout)"
    } else {
        ""
    };

    // Top subplot occupies y [0.55, 1.0], bottom occupies y [0.0, 0.45]
    let layout = Layout::new()
        .title(format!(
            "HIT-Leiden Incremental Benchmark — {}{}",
            run.dataset_id, truncated_note
        ))
        // Top subplot axes
        .x_axis(Axis::new().title("Batch").domain(&[0.0, 1.0]).anchor("y"))
        .y_axis(
            Axis::new()
                .title("Time (ms)")
                .type_(AxisType::Log)
                .domain(&[0.55, 1.0])
                .anchor("x"),
        )
        .y_axis2(
            Axis::new()
                .title("Speedup (x)")
                .overlaying("y")
                .side(AxisSide::Right)
                .anchor("x"),
        )
        // Bottom subplot axes
        .x_axis2(Axis::new().title("Batch").domain(&[0.0, 1.0]).anchor("y3"))
        .y_axis3(
            Axis::new()
                .title("Modularity (Q)")
                .domain(&[0.0, 0.45])
                .anchor("x2"),
        )
        .legend(Legend::new().x(1.05).y(1.0))
        .height(800);
    plot.set_layout(layout);

    plot.write_html(output_path);
    augment_html_report(run, output_path)?;

    Ok(())
}

fn augment_html_report(
    run: &BenchmarkRun,
    output_path: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut html = fs::read_to_string(output_path)?;
    let report_sections = build_report_sections(run, output_path);

    if let Some(pos) = html.rfind("</body>") {
        html.insert_str(pos, &report_sections);
    } else {
        html.push_str(&report_sections);
    }

    fs::write(output_path, html)?;
    Ok(())
}

fn build_report_sections(run: &BenchmarkRun, output_path: &Path) -> String {
    let mut section = String::new();
    section.push_str(
        r#"
<section style="margin: 28px auto; max-width: 1200px; font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Arial, sans-serif; color: #222;">
  <h2 style="margin: 0 0 12px;">Benchmark report details</h2>
  <div style="display: flex; flex-wrap: wrap; gap: 16px; margin-bottom: 16px;">
    <div><strong>Dataset:</strong> "#,
    );
    section.push_str(&xml_escape(&run.dataset_id));
    section.push_str("</div><div><strong>Timestamp:</strong> ");
    section.push_str(&xml_escape(&run.timestamp));
    section.push_str("</div><div><strong>Truncated:</strong> ");
    section.push_str(if run.truncated { "yes" } else { "no" });
    section.push_str("</div></div>");

    section.push_str(&render_batch_results_table(run));
    section.push_str(&render_final_comparison_table(run));
    section.push_str(&render_embedded_svg(output_path));
    section.push_str("</section>\n");
    section
}

fn render_batch_results_table(run: &BenchmarkRun) -> String {
    let has_igraph = run.batches.iter().any(|b| b.igraph_time_ms > 0.0);
    let th = |text: &str| -> String {
        format!(
            "<th style=\"text-align:right;padding:8px;border:1px solid #d0d7de;\">{}</th>",
            text
        )
    };
    let td_right = |text: String| -> String {
        format!(
            "<td style=\"padding:8px;border:1px solid #d0d7de;text-align:right;\">{}</td>",
            text
        )
    };

    let mut table = String::new();
    table.push_str(
        r#"
  <h3 style="margin: 20px 0 10px;">Batch results</h3>
  <div style="overflow-x: auto; margin-bottom: 18px;">
    <table style="border-collapse: collapse; min-width: 980px; width: 100%;">
      <thead>
        <tr style="background: #f6f8fa;">
          <th style="text-align:left;padding:8px;border:1px solid #d0d7de;">Batch</th>
"#,
    );
    table.push_str(&th("Total edges"));
    table.push_str(&th("HIT (ms)"));
    if has_igraph {
        table.push_str(&th("igraph (ms)"));
        table.push_str(&th("Speedup (x)"));
        table.push_str(&th("Cum. Speedup (x)"));
    }
    table.push_str(&th("HIT Q"));
    if has_igraph {
        table.push_str(&th("igraph Q"));
    }
    table.push_str(
        r#"
        </tr>
      </thead>
      <tbody>
"#,
    );

    let mut cumulative_hit = 0.0_f64;
    let mut cumulative_ig = 0.0_f64;

    for batch in &run.batches {
        cumulative_hit += batch.hit_leiden_time_ms;
        cumulative_ig += batch.igraph_time_ms;
        let cumulative_speedup = if cumulative_hit > 0.0 && cumulative_ig > 0.0 {
            cumulative_ig / cumulative_hit
        } else {
            0.0
        };

        table.push_str("        <tr>\n");
        table.push_str(&format!(
            "          <td style=\"padding:8px;border:1px solid #d0d7de;\">{}</td>\n",
            batch.batch_idx + 1
        ));
        table.push_str(&format!(
            "          {}\n",
            td_right(format!("{}", batch.total_edges))
        ));
        table.push_str(&format!(
            "          {}\n",
            td_right(format!("{:.2}", batch.hit_leiden_time_ms))
        ));
        if has_igraph {
            table.push_str(&format!(
                "          {}\n",
                td_right(format!("{:.2}", batch.igraph_time_ms))
            ));
            table.push_str(&format!(
                "          {}\n",
                td_right(format!("{:.2}x", batch.speedup))
            ));
            table.push_str(&format!(
                "          {}\n",
                td_right(format!("{:.2}x", cumulative_speedup))
            ));
        }
        table.push_str(&format!(
            "          {}\n",
            td_right(format!("{:.5}", batch.modularity))
        ));
        if has_igraph {
            table.push_str(&format!(
                "          {}\n",
                td_right(format!("{:.5}", batch.igraph_modularity))
            ));
        }
        table.push_str("        </tr>\n");
    }

    table.push_str(
        r#"      </tbody>
    </table>
  </div>
"#,
    );
    table
}

fn render_final_comparison_table(run: &BenchmarkRun) -> String {
    let Some(final_cmp) = run.final_batch_comparison.as_ref() else {
        return String::new();
    };

    let row = |label: &str, value: &str| -> String {
        format!(
            "        <tr><th style=\"text-align:left;padding:8px;border:1px solid #d0d7de;background:#f6f8fa;\">{}</th><td style=\"padding:8px;border:1px solid #d0d7de;\">{}</td></tr>\n",
            label, value
        )
    };

    let mut table = String::new();
    table.push_str(
        r#"
  <h3 style="margin: 20px 0 10px;">Final batch fresh comparison</h3>
  <div style="overflow-x: auto; margin-bottom: 18px;">
    <table style="border-collapse: collapse; min-width: 820px; width: 100%;">
      <tbody>
"#,
    );

    table.push_str(&row("Batch", &format!("{}", final_cmp.batch_idx + 1)));
    table.push_str(&row(
        "HIT time (ms)",
        &format!("{:.2}", final_cmp.hit_time_ms),
    ));
    table.push_str(&row(
        "Fresh igraph time (ms)",
        &format!("{:.2}", final_cmp.igraph_fresh_time_ms),
    ));
    table.push_str(&row(
        "Speedup vs fresh igraph (x)",
        &format!("{:.2}x", final_cmp.speedup_vs_fresh_igraph),
    ));
    table.push_str(&row(
        "Modularity HIT",
        &format!("{:.5}", final_cmp.hit_modularity),
    ));
    table.push_str(&row(
        "Modularity igraph",
        &format!("{:.5}", final_cmp.igraph_fresh_modularity),
    ));
    table.push_str(&row(
        "Modularity \u{0394} (HIT\u{2212}igraph)",
        &format!("{:+.5}", final_cmp.modularity_delta),
    ));
    table.push_str(&row(
        "NMI (HIT vs igraph)",
        &format!("{:.4}", final_cmp.nmi),
    ));
    table.push_str(&row(
        "HIT \u{2192} igraph purity",
        &format!("{:.4}", final_cmp.hit_to_igraph_purity),
    ));
    table.push_str(&row(
        "igraph \u{2192} HIT purity",
        &format!("{:.4}", final_cmp.igraph_to_hit_purity),
    ));
    table.push_str(&row(
        "Largest-community Jaccard",
        &format!("{:.4}", final_cmp.largest_community_jaccard),
    ));
    table.push_str(&row(
        "igraph community count",
        &format!("{}", final_cmp.igraph_community_count),
    ));

    table.push_str(
        r#"      </tbody>
    </table>
  </div>
"#,
    );

    table
}

fn render_embedded_svg(output_path: &Path) -> String {
    let Some(svg_path) = infer_community_svg_path(output_path) else {
        return "<h3 style=\"margin:20px 0 10px;\">Community structure</h3><p>Community SVG path could not be inferred for this report.</p>".to_string();
    };

    match fs::read_to_string(&svg_path) {
        Ok(svg) => format!(
            r#"
  <h3 style="margin: 20px 0 10px;">Community structure (embedded SVG)</h3>
  <div style="border:1px solid #d0d7de; border-radius:8px; padding:8px; background:#fff; overflow:auto;">
    {}
  </div>
"#,
            svg
        ),
        Err(_) => format!(
            r#"
  <h3 style="margin: 20px 0 10px;">Community structure</h3>
  <p>Community SVG not available at report generation time: <code>{}</code></p>
"#,
            xml_escape(&svg_path.display().to_string())
        ),
    }
}

fn infer_community_svg_path(output_path: &Path) -> Option<PathBuf> {
    let parent = output_path.parent()?;
    let filename = output_path.file_name()?.to_str()?;
    if !filename.starts_with("benchmark_") || !filename.ends_with(".html") {
        return None;
    }
    let stem = filename
        .trim_start_matches("benchmark_")
        .trim_end_matches(".html");
    Some(parent.join(format!("communities_{}.svg", stem)))
}

// -- Community structure SVG chart --

/// Color palette for communities: (hull_fill, node_fill)
const COMMUNITY_PALETTE: &[(&str, &str)] = &[
    ("#aec7e8", "#1f77b4"), // blue
    ("#ffbb78", "#ff7f0e"), // orange
    ("#98df8a", "#2ca02c"), // green
    ("#ff9896", "#d62728"), // red
    ("#c5b0d5", "#9467bd"), // purple
    ("#c49c94", "#8c564b"), // brown
    ("#f7b6d2", "#e377c2"), // pink
    ("#dbdb8d", "#bcbd22"), // yellow-green
    ("#9edae5", "#17becf"), // cyan
    ("#c7c7c7", "#7f7f7f"), // gray
    ("#d4b9da", "#6a3d9a"), // violet
    ("#fdbf6f", "#e67300"), // dark orange
    ("#b2df8a", "#33a02c"), // lime
    ("#fb9a99", "#e31a1c"), // crimson
    ("#a6cee3", "#1f78b4"), // steel blue
    ("#fddbc7", "#b35806"), // amber
    ("#d9f0d3", "#1b7837"), // forest
    ("#fee0b6", "#d94801"), // rust
    ("#c6dbef", "#2171b5"), // sky
    ("#fcbba1", "#cb181d"), // coral
];

const HIERARCHY_TOP_K: usize = 6;
const HIERARCHY_HULL_PADDING: f64 = 14.0;
const HIERARCHY_STROKE_WIDTH: f64 = 1.2;
const HIERARCHY_STROKE_OPACITY: f64 = 0.45;
const HIERARCHY_FILL_OPACITY: f64 = 0.015;

/// Generate a community structure chart as SVG with force-directed layout.
/// Uses a single shared layout so both panels have identical node positions,
/// making it easy to see where HIT-Leiden and igraph agree or differ.
pub fn generate_community_chart(
    hit_partition: &[usize],
    hit_hierarchy_levels: Option<&[Vec<usize>]>,
    igraph_partition: Option<&[usize]>,
    edges: &[(usize, usize, Option<f64>)],
    dataset_id: &str,
    output_path: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    if let Some(parent) = output_path.parent() {
        fs::create_dir_all(parent)?;
    }

    let num_nodes = hit_partition.len();
    let has_igraph = igraph_partition.is_some();

    let panel_w = 800.0;
    let panel_h = 800.0;
    let gap = 40.0;
    let title_h = 50.0;
    let legend_w = 180.0;
    let total_w = if has_igraph {
        panel_w * 2.0 + gap + legend_w
    } else {
        panel_w + legend_w
    };
    let total_h = panel_h + title_h;

    let mut svg = format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {total_w} {total_h}" width="{total_w}" height="{total_h}">
<style>
  text {{ font-family: 'Helvetica Neue', Arial, sans-serif; }}
  .title {{ font-size: 18px; font-weight: bold; text-anchor: middle; }}
  .panel-label {{ font-size: 14px; font-weight: bold; text-anchor: middle; fill: #333; }}
  .legend-text {{ font-size: 11px; fill: #333; }}
  .legend-title {{ font-size: 12px; font-weight: bold; fill: #333; }}
</style>
<rect width="{total_w}" height="{total_h}" fill="white"/>
"#
    );

    // Title
    let hit_comm_count = group_by_community(hit_partition).len();
    let ig_comm_count = igraph_partition.map(|p| group_by_community(p).len());
    let hit_hierarchy_overlay = select_hierarchy_overlay(hit_partition, hit_hierarchy_levels);
    let hit_super_partition = hit_hierarchy_overlay.as_ref().map(|s| s.partition.clone());
    let hit_super_count = hit_super_partition
        .as_ref()
        .map(|p| group_by_community(p).len());
    let overlay_label = hit_hierarchy_overlay.as_ref().map(|s| {
        if s.is_super {
            "super-communities"
        } else {
            "hierarchy groups"
        }
    });
    let title = if let Some(ig_c) = ig_comm_count {
        if let (Some(sc), Some(label)) = (hit_super_count, overlay_label) {
            format!(
                "Community Structure \u{2014} {} | HIT-Leiden ({} communities, {} {}) vs igraph ({} communities)",
                dataset_id, hit_comm_count, sc, label, ig_c
            )
        } else {
            format!(
                "Community Structure \u{2014} {} | HIT-Leiden ({} communities) vs igraph ({} communities)",
                dataset_id, hit_comm_count, ig_c
            )
        }
    } else if let (Some(sc), Some(label)) = (hit_super_count, overlay_label) {
        format!(
            "Community Structure \u{2014} {} | HIT-Leiden ({} communities, {} {})",
            dataset_id, hit_comm_count, sc, label
        )
    } else {
        format!(
            "Community Structure \u{2014} {} | HIT-Leiden ({} communities)",
            dataset_id, hit_comm_count
        )
    };
    let content_w = if has_igraph {
        panel_w * 2.0 + gap
    } else {
        panel_w
    };
    svg.push_str(&format!(
        r#"<text x="{}" y="32" class="title">{}</text>
"#,
        content_w / 2.0,
        xml_escape(&title)
    ));

    // Compute a SINGLE shared layout from the HIT-Leiden partition.
    eprintln!("Computing shared force-directed layout...");
    let (shared_x, shared_y) = community_force_layout(hit_partition, edges, num_nodes, 42);

    let hit_top = top_communities(hit_partition, COMMUNITY_PALETTE.len());

    render_panel(
        &mut svg,
        &PanelParams {
            x: &shared_x,
            y: &shared_y,
            partition: hit_partition,
            edges,
            num_nodes,
            offset_x: 0.0,
            offset_y: title_h,
            width: panel_w,
            height: panel_h,
            margin: 50.0,
            label: "HIT-Leiden",
            top_comms: &hit_top,
            super_partition: hit_super_partition.as_deref(),
        },
    );

    if let Some(ig_part) = igraph_partition {
        let ig_top = top_communities(ig_part, COMMUNITY_PALETTE.len());
        render_panel(
            &mut svg,
            &PanelParams {
                x: &shared_x,
                y: &shared_y,
                partition: ig_part,
                edges,
                num_nodes,
                offset_x: panel_w + gap,
                offset_y: title_h,
                width: panel_w,
                height: panel_h,
                margin: 50.0,
                label: "igraph",
                top_comms: &ig_top,
                super_partition: None,
            },
        );
    }

    // Legend
    let legend_x = content_w + 20.0;
    let mut legend_y = title_h + 20.0;
    svg.push_str(&format!(
        r#"<text x="{}" y="{}" class="legend-title">HIT-Leiden Top</text>
"#,
        legend_x,
        legend_y + 12.0
    ));
    for (i, &(comm_id, size)) in hit_top.iter().enumerate().take(10) {
        let iy = legend_y + 30.0 + i as f64 * 18.0;
        let (fill, node_color) = COMMUNITY_PALETTE[i % COMMUNITY_PALETTE.len()];
        svg.push_str(&format!(
            r#"<rect x="{}" y="{}" width="12" height="12" rx="2" fill="{}" fill-opacity="0.4" stroke="{}" stroke-width="1"/>
<circle cx="{}" cy="{}" r="4" fill="{}"/>
<text x="{}" y="{}" class="legend-text">C{} ({} nodes)</text>
"#,
            legend_x, iy, fill, node_color,
            legend_x + 6.0, iy + 6.0, node_color,
            legend_x + 18.0, iy + 10.0, comm_id, size,
        ));
    }

    if let Some(ig_part) = igraph_partition {
        legend_y += 30.0 + 10.0 * 18.0 + 20.0;
        let ig_top = top_communities(ig_part, COMMUNITY_PALETTE.len());
        svg.push_str(&format!(
            r#"<text x="{}" y="{}" class="legend-title">igraph Top</text>
"#,
            legend_x,
            legend_y + 12.0
        ));
        for (i, &(comm_id, size)) in ig_top.iter().enumerate().take(10) {
            let iy = legend_y + 30.0 + i as f64 * 18.0;
            let (fill, node_color) = COMMUNITY_PALETTE[i % COMMUNITY_PALETTE.len()];
            svg.push_str(&format!(
                r#"<rect x="{}" y="{}" width="12" height="12" rx="2" fill="{}" fill-opacity="0.4" stroke="{}" stroke-width="1"/>
<circle cx="{}" cy="{}" r="4" fill="{}"/>
<text x="{}" y="{}" class="legend-text">C{} ({} nodes)</text>
"#,
                legend_x, iy, fill, node_color,
                legend_x + 6.0, iy + 6.0, node_color,
                legend_x + 18.0, iy + 10.0, comm_id, size,
            ));
        }
    }

    if let Some(super_part) = hit_super_partition.as_deref() {
        legend_y += 30.0 + 10.0 * 18.0 + 20.0;
        let super_top = top_communities(super_part, HIERARCHY_TOP_K);
        let hierarchy_title = if hit_hierarchy_overlay
            .as_ref()
            .map(|s| s.is_super)
            .unwrap_or(false)
        {
            "HIT Super Top"
        } else {
            "HIT Hierarchy Top"
        };
        svg.push_str(&format!(
            r#"<text x="{}" y="{}" class="legend-title">{}</text>
"#,
            legend_x,
            legend_y + 12.0,
            hierarchy_title
        ));
        for (i, &(comm_id, size)) in super_top.iter().enumerate().take(HIERARCHY_TOP_K) {
            let iy = legend_y + 30.0 + i as f64 * 18.0;
            svg.push_str(&format!(
            r##"<rect x="{}" y="{}" width="12" height="12" rx="2" fill="#000" fill-opacity="0.02" stroke="#111" stroke-width="1.0" stroke-opacity="0.5" stroke-dasharray="4 2"/>
<text x="{}" y="{}" class="legend-text">S{} ({} nodes)</text>
"##,
                legend_x,
                iy,
                legend_x + 18.0,
                iy + 10.0,
                comm_id,
                size,
            ));
        }
    }

    svg.push_str("</svg>\n");
    fs::write(output_path, svg)?;
    Ok(())
}

fn top_communities(partition: &[usize], n: usize) -> Vec<(usize, usize)> {
    let groups = group_by_community(partition);
    let mut sized: Vec<(usize, usize)> = groups.iter().map(|(id, m)| (*id, m.len())).collect();
    sized.sort_by(|a, b| b.1.cmp(&a.1));
    sized.truncate(n);
    sized
}

fn group_by_community(partition: &[usize]) -> Vec<(usize, Vec<usize>)> {
    let mut map: HashMap<usize, Vec<usize>> = HashMap::new();
    for (node, &comm) in partition.iter().enumerate() {
        map.entry(comm).or_default().push(node);
    }
    let mut groups: Vec<(usize, Vec<usize>)> = map.into_iter().collect();
    groups.sort_by_key(|(id, _)| *id);
    groups
}

fn community_force_layout(
    partition: &[usize],
    edges: &[(usize, usize, Option<f64>)],
    num_nodes: usize,
    seed: u64,
) -> (Vec<f64>, Vec<f64>) {
    let communities = group_by_community(partition);
    let num_comm = communities.len();

    if num_comm == 0 {
        return (vec![0.0; num_nodes], vec![0.0; num_nodes]);
    }

    let comm_id_map: HashMap<usize, usize> = communities
        .iter()
        .enumerate()
        .map(|(idx, (comm_id, _))| (*comm_id, idx))
        .collect();

    let mut inter_edges_map: HashMap<(usize, usize), f64> = HashMap::new();
    for &(src, dst, w) in edges {
        if src >= num_nodes || dst >= num_nodes {
            continue;
        }
        let cs = comm_id_map[&partition[src]];
        let cd = comm_id_map[&partition[dst]];
        if cs != cd {
            let key = if cs < cd { (cs, cd) } else { (cd, cs) };
            *inter_edges_map.entry(key).or_default() += w.unwrap_or(1.0);
        }
    }
    let comm_edges: Vec<(usize, usize, f64)> = inter_edges_map
        .into_iter()
        .map(|((a, b), w)| (a, b, w))
        .collect();

    let comm_sizes: Vec<usize> = communities.iter().map(|(_, m)| m.len()).collect();

    let (comm_x, comm_y) =
        fruchterman_reingold(num_comm, &comm_edges, &comm_sizes, 1000.0, 1000.0, 80, seed);

    let max_size = *comm_sizes.iter().max().unwrap_or(&1) as f64;
    let mut x = vec![0.0f64; num_nodes];
    let mut y = vec![0.0f64; num_nodes];

    for (idx, (_, members)) in communities.iter().enumerate() {
        let cx = comm_x[idx];
        let cy = comm_y[idx];

        if members.len() == 1 {
            x[members[0]] = cx;
            y[members[0]] = cy;
        } else {
            let r = 5.0 + 30.0 * (members.len() as f64 / max_size).sqrt();
            for (ni, &node) in members.iter().enumerate() {
                let angle = 2.0 * std::f64::consts::PI * ni as f64 / members.len() as f64;
                x[node] = cx + r * angle.cos();
                y[node] = cy + r * angle.sin();
            }
        }
    }

    (x, y)
}

fn fruchterman_reingold(
    num_nodes: usize,
    edges: &[(usize, usize, f64)],
    sizes: &[usize],
    width: f64,
    height: f64,
    iterations: usize,
    seed: u64,
) -> (Vec<f64>, Vec<f64>) {
    use rand::Rng;
    use rand::SeedableRng;

    let mut rng = rand::rngs::StdRng::seed_from_u64(seed);
    let area = width * height;
    let k = (area / num_nodes.max(1) as f64).sqrt();

    let mut x: Vec<f64> = (0..num_nodes).map(|_| rng.gen_range(0.0..width)).collect();
    let mut y: Vec<f64> = (0..num_nodes).map(|_| rng.gen_range(0.0..height)).collect();

    let mut temp = width / 10.0;
    let cool_factor = temp / iterations.max(1) as f64;

    for _ in 0..iterations {
        let mut dx = vec![0.0f64; num_nodes];
        let mut dy = vec![0.0f64; num_nodes];

        for i in 0..num_nodes {
            for j in (i + 1)..num_nodes {
                let ddx = x[i] - x[j];
                let ddy = y[i] - y[j];
                let dist = (ddx * ddx + ddy * ddy).sqrt().max(0.1);
                let weight = ((sizes[i] + sizes[j]) as f64).sqrt();
                let force = k * k / dist * weight;
                let fx = ddx / dist * force;
                let fy = ddy / dist * force;
                dx[i] += fx;
                dy[i] += fy;
                dx[j] -= fx;
                dy[j] -= fy;
            }
        }

        for &(u, v, w) in edges {
            let ddx = x[u] - x[v];
            let ddy = y[u] - y[v];
            let dist = (ddx * ddx + ddy * ddy).sqrt().max(0.1);
            let force = dist * dist / k * w.sqrt().max(0.1);
            let fx = ddx / dist * force;
            let fy = ddy / dist * force;
            dx[u] -= fx;
            dy[u] -= fy;
            dx[v] += fx;
            dy[v] += fy;
        }

        for i in 0..num_nodes {
            let disp = (dx[i] * dx[i] + dy[i] * dy[i]).sqrt().max(0.1);
            let capped = disp.min(temp);
            x[i] += dx[i] / disp * capped;
            y[i] += dy[i] / disp * capped;
            x[i] = x[i].clamp(0.0, width);
            y[i] = y[i].clamp(0.0, height);
        }

        temp -= cool_factor;
        if temp < 0.0 {
            temp = 0.0;
        }
    }

    (x, y)
}

struct PanelParams<'a> {
    x: &'a [f64],
    y: &'a [f64],
    partition: &'a [usize],
    edges: &'a [(usize, usize, Option<f64>)],
    num_nodes: usize,
    offset_x: f64,
    offset_y: f64,
    width: f64,
    height: f64,
    margin: f64,
    label: &'a str,
    top_comms: &'a [(usize, usize)],
    super_partition: Option<&'a [usize]>,
}

fn render_panel(svg: &mut String, p: &PanelParams<'_>) {
    let PanelParams {
        x,
        y,
        partition,
        edges,
        num_nodes,
        offset_x,
        offset_y,
        width,
        height,
        margin,
        label,
        top_comms,
        super_partition,
    } = *p;
    let top_comm_colors: HashMap<usize, usize> = top_comms
        .iter()
        .enumerate()
        .map(|(color_idx, &(comm_id, _))| (comm_id, color_idx))
        .collect();

    let (min_x, max_x, min_y, max_y) = bounding_box(x, y, num_nodes);
    let range_x = (max_x - min_x).max(1.0);
    let range_y = (max_y - min_y).max(1.0);
    let scale = ((width - 2.0 * margin) / range_x).min((height - 2.0 * margin - 20.0) / range_y);
    let cx_off = offset_x + margin + (width - 2.0 * margin - range_x * scale) / 2.0;
    let cy_off = offset_y + margin + 20.0 + (height - 2.0 * margin - 20.0 - range_y * scale) / 2.0;

    let tx = |px: f64| -> f64 { cx_off + (px - min_x) * scale };
    let ty = |py: f64| -> f64 { cy_off + (py - min_y) * scale };

    svg.push_str(&format!(
        r##"<rect x="{}" y="{}" width="{}" height="{}" fill="white" stroke="#ccc" stroke-width="1" rx="4"/>
"##,
        offset_x, offset_y, width, height
    ));

    svg.push_str(&format!(
        r#"<text x="{}" y="{}" class="panel-label">{}</text>
"#,
        offset_x + width / 2.0,
        offset_y + 18.0,
        label
    ));

    let communities = group_by_community(partition);
    for &(comm_id, _) in top_comms {
        if let Some(color_idx) = top_comm_colors.get(&comm_id) {
            let members: Vec<usize> = communities
                .iter()
                .find(|(id, _)| *id == comm_id)
                .map(|(_, m)| m.clone())
                .unwrap_or_default();
            if members.len() < 3 {
                continue;
            }
            let points: Vec<(f64, f64)> = members.iter().map(|&i| (tx(x[i]), ty(y[i]))).collect();
            let hull = convex_hull(&points);
            if hull.len() < 3 {
                continue;
            }
            let padded = pad_hull(&hull, 12.0);
            let path_d = smooth_hull_path(&padded);
            let (fill, stroke) = COMMUNITY_PALETTE[*color_idx % COMMUNITY_PALETTE.len()];
            svg.push_str(&format!(
                r#"<path d="{}" fill="{}" fill-opacity="0.2" stroke="{}" stroke-opacity="0.5" stroke-width="1.5"/>
"#,
                path_d, fill, stroke
            ));
        }
    }

    if let Some(super_part) = super_partition {
        let super_top = top_communities(super_part, HIERARCHY_TOP_K);
        let super_groups = group_by_community(super_part);
        for &(super_id, _) in &super_top {
            let members: Vec<usize> = super_groups
                .iter()
                .find(|(id, _)| *id == super_id)
                .map(|(_, m)| m.clone())
                .unwrap_or_default();
            if members.len() < 3 {
                continue;
            }
            let points: Vec<(f64, f64)> = members.iter().map(|&i| (tx(x[i]), ty(y[i]))).collect();
            let hull = convex_hull(&points);
            if hull.len() < 3 {
                continue;
            }
            let padded = pad_hull(&hull, HIERARCHY_HULL_PADDING);
            let path_d = smooth_hull_path(&padded);
            svg.push_str(&format!(
                r##"<path d="{}" fill="#000" fill-opacity="{}" stroke="#111" stroke-opacity="{}" stroke-width="{}" stroke-dasharray="4 3"/>
"##,
                path_d,
                HIERARCHY_FILL_OPACITY,
                HIERARCHY_STROKE_OPACITY,
                HIERARCHY_STROKE_WIDTH
            ));
        }
    }

    svg.push_str(
        r##"<path fill="none" stroke="#888" stroke-opacity="0.08" stroke-width="0.3" d=""##,
    );
    for &(src, dst, _) in edges {
        if src < num_nodes && dst < num_nodes {
            svg.push_str(&format!(
                "M{:.1},{:.1}L{:.1},{:.1}",
                tx(x[src]),
                ty(y[src]),
                tx(x[dst]),
                ty(y[dst])
            ));
        }
    }
    svg.push_str(r#""/>"#);
    svg.push('\n');

    svg.push_str(r##"<g fill="#bbb" opacity="0.6">"##);
    for i in 0..num_nodes {
        if !top_comm_colors.contains_key(&partition[i]) {
            svg.push_str(&format!(
                r#"<circle cx="{:.1}" cy="{:.1}" r="1.5"/>"#,
                tx(x[i]),
                ty(y[i])
            ));
        }
    }
    svg.push_str("</g>\n");

    for &(comm_id, _) in top_comms {
        let Some(&color_idx) = top_comm_colors.get(&comm_id) else {
            continue;
        };
        let (_, node_color) = COMMUNITY_PALETTE[color_idx % COMMUNITY_PALETTE.len()];
        svg.push_str(&format!(r#"<g fill="{}" opacity="0.85">"#, node_color));
        for i in (0..num_nodes).filter(|&i| partition[i] == comm_id) {
            svg.push_str(&format!(
                r#"<circle cx="{:.1}" cy="{:.1}" r="2.5"/>"#,
                tx(x[i]),
                ty(y[i])
            ));
        }
        svg.push_str("</g>\n");
    }
}

// -- Geometry helpers --

fn convex_hull(points: &[(f64, f64)]) -> Vec<(f64, f64)> {
    let mut pts = points.to_vec();
    pts.sort_by(|a, b| {
        a.0.partial_cmp(&b.0)
            .unwrap()
            .then(a.1.partial_cmp(&b.1).unwrap())
    });

    if pts.len() <= 2 {
        return pts;
    }

    let mut lower = Vec::new();
    for &p in &pts {
        while lower.len() >= 2 {
            let a = lower[lower.len() - 2];
            let b = lower[lower.len() - 1];
            if cross_product(a, b, p) <= 0.0 {
                lower.pop();
            } else {
                break;
            }
        }
        lower.push(p);
    }

    let mut upper = Vec::new();
    for &p in pts.iter().rev() {
        while upper.len() >= 2 {
            let a = upper[upper.len() - 2];
            let b = upper[upper.len() - 1];
            if cross_product(a, b, p) <= 0.0 {
                upper.pop();
            } else {
                break;
            }
        }
        upper.push(p);
    }

    lower.pop();
    upper.pop();
    lower.extend(upper);
    lower
}

fn cross_product(o: (f64, f64), a: (f64, f64), b: (f64, f64)) -> f64 {
    (a.0 - o.0) * (b.1 - o.1) - (a.1 - o.1) * (b.0 - o.0)
}

fn pad_hull(hull: &[(f64, f64)], padding: f64) -> Vec<(f64, f64)> {
    if hull.is_empty() {
        return vec![];
    }
    let n = hull.len() as f64;
    let cx = hull.iter().map(|p| p.0).sum::<f64>() / n;
    let cy = hull.iter().map(|p| p.1).sum::<f64>() / n;
    hull.iter()
        .map(|&(px, py)| {
            let dx = px - cx;
            let dy = py - cy;
            let d = (dx * dx + dy * dy).sqrt().max(0.01);
            (px + dx / d * padding, py + dy / d * padding)
        })
        .collect()
}

fn smooth_hull_path(points: &[(f64, f64)]) -> String {
    if points.is_empty() {
        return String::new();
    }
    if points.len() < 3 {
        let mut d = format!("M{:.1},{:.1}", points[0].0, points[0].1);
        for p in &points[1..] {
            d.push_str(&format!("L{:.1},{:.1}", p.0, p.1));
        }
        d.push('Z');
        return d;
    }

    let n = points.len();
    let mut d = format!("M{:.1},{:.1}", points[0].0, points[0].1);

    for i in 0..n {
        let p0 = points[(i + n - 1) % n];
        let p1 = points[i];
        let p2 = points[(i + 1) % n];
        let p3 = points[(i + 2) % n];

        let cp1x = p1.0 + (p2.0 - p0.0) / 6.0;
        let cp1y = p1.1 + (p2.1 - p0.1) / 6.0;
        let cp2x = p2.0 - (p3.0 - p1.0) / 6.0;
        let cp2y = p2.1 - (p3.1 - p1.1) / 6.0;

        d.push_str(&format!(
            "C{:.1},{:.1} {:.1},{:.1} {:.1},{:.1}",
            cp1x, cp1y, cp2x, cp2y, p2.0, p2.1
        ));
    }

    d.push('Z');
    d
}

fn bounding_box(x: &[f64], y: &[f64], num_nodes: usize) -> (f64, f64, f64, f64) {
    let mut min_x = f64::MAX;
    let mut max_x = f64::MIN;
    let mut min_y = f64::MAX;
    let mut max_y = f64::MIN;
    for i in 0..num_nodes {
        min_x = min_x.min(x[i]);
        max_x = max_x.max(x[i]);
        min_y = min_y.min(y[i]);
        max_y = max_y.max(y[i]);
    }
    (min_x, max_x, min_y, max_y)
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

struct HierarchyOverlaySelection {
    partition: Vec<usize>,
    is_super: bool,
}

fn select_hierarchy_overlay(
    hit_partition: &[usize],
    hit_hierarchy_levels: Option<&[Vec<usize>]>,
) -> Option<HierarchyOverlaySelection> {
    let levels = hit_hierarchy_levels?;
    if levels.is_empty() {
        return None;
    }

    let base_finest_count = group_by_community(&levels[0]).len();
    let current_count = group_by_community(hit_partition).len();

    let mut candidates: Vec<(usize, usize, &Vec<usize>)> = levels
        .iter()
        .enumerate()
        .filter(|(_, level)| {
            level.len() == hit_partition.len() && level.as_slice() != hit_partition
        })
        .map(|(idx, level)| (idx, group_by_community(level).len(), level))
        .collect();

    if candidates.is_empty() {
        return None;
    }

    let mut true_super: Vec<(usize, usize, &Vec<usize>)> = candidates
        .iter()
        .copied()
        .filter(|(_, count, _)| *count < current_count)
        .collect();

    if !true_super.is_empty() {
        true_super.sort_by(|a, b| a.1.cmp(&b.1).then(a.0.cmp(&b.0)));
        let (_, _, selected) = true_super[0];
        return Some(HierarchyOverlaySelection {
            partition: selected.clone(),
            is_super: true,
        });
    }

    candidates.sort_by(|a, b| {
        let a_pref = (a.1 >= base_finest_count) as u8;
        let b_pref = (b.1 >= base_finest_count) as u8;
        a_pref.cmp(&b_pref).then(a.1.cmp(&b.1)).then(a.0.cmp(&b.0))
    });
    let (_, _, selected) = candidates[0];
    Some(HierarchyOverlaySelection {
        partition: selected.clone(),
        is_super: false,
    })
}
