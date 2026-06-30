use serde_json::json;

use crate::db::links::Graph;

use super::layout::{HeadMeta, SiteMeta, escape_html, json_for_script, render_page};

/// Render the `/graph` page: an interactive, client-side force-directed view of
/// the whole garden's link graph (pan, zoom, drag, hover-highlight, click-to-
/// open), with a no-JS fallback list so the page is useful without scripting.
///
/// The graph data is serialized into a `<script type="application/json">` island
/// (escaped via [`json_for_script`], so a note title containing `</script>`
/// cannot break out) and parsed by a single nonce'd behavior script — no values
/// are interpolated into executable JS. The SVG canvas is populated entirely by
/// that script; with JS off, the canvas stays hidden and the fallback `<ul>`
/// (server-rendered) is what the visitor sees. An empty graph degrades to a
/// plain message with no canvas or script.
pub fn render_graph_page(graph: &Graph, csp_nonce: &str, site: &SiteMeta<'_>) -> String {
    let header = r#"<div class="graph-page-header">
          <h1>Graph <span class="accent-dot">·</span> <span class="accent-title">Garden View</span></h1>
          <p class="graph-subtitle">Every note and the links between them. Drag to pan, scroll to zoom, click a node to open it.</p>
        </div>"#;

    let main = if graph.nodes.is_empty() {
        format!(
            r#"{header}
        <p class="empty">No published notes to graph yet.</p>"#
        )
    } else {
        // No-JS fallback: a plain list of every node, server-rendered. Visible by
        // default; CSS hides it once the behavior script flips `js-graph-active`.
        let fallback = graph
            .nodes
            .iter()
            .map(|node| {
                format!(
                    r#"<li><a href="/{slug}">{title}</a></li>"#,
                    slug = urlencoding::encode(&node.slug),
                    title = escape_html(&node.title),
                )
            })
            .collect::<Vec<_>>()
            .join("\n");

        // Data island. Keys are short (`s`/`t` for edges) to keep the payload
        // small; values are the only attacker-influenced content and are escaped
        // by `json_for_script`, so `</script>`/`<`/`&` cannot break the tag.
        let payload = json!({
            "nodes": graph
                .nodes
                .iter()
                .map(|n| json!({ "slug": n.slug, "title": n.title }))
                .collect::<Vec<_>>(),
            "edges": graph
                .edges
                .iter()
                .map(|e| json!({ "s": e.source_slug, "t": e.target_slug }))
                .collect::<Vec<_>>(),
        });
        let data_island = json_for_script(payload);
        let nonce = escape_html(csp_nonce);

        format!(
            r#"{header}
        <ul class="graph-fallback">
{fallback}
        </ul>
        <div class="graph-canvas" aria-hidden="true">
          <svg class="graph-svg" viewBox="0 0 1000 700" preserveAspectRatio="xMidYMid meet" role="img" aria-label="Garden link graph">
            <g class="graph-viewport"></g>
          </svg>
        </div>
        <script type="application/json" id="graph-data" nonce="{nonce}">{data_island}</script>
        <script nonce="{nonce}">
(function () {{
  var el = document.getElementById('graph-data');
  var svg = document.querySelector('.graph-svg');
  var viewport = document.querySelector('.graph-viewport');
  if (!el || !svg || !viewport) return;
  var data;
  try {{ data = JSON.parse(el.textContent); }} catch (e) {{ return; }}
  if (!data || !data.nodes || !data.nodes.length) return;

  document.documentElement.classList.add('js-graph-active');

  var SVGNS = 'http://www.w3.org/2000/svg';
  var W = 1000, H = 700, CX = W / 2, CY = H / 2;
  var n = data.nodes.length;

  // Deterministic seed: each node on a circle by index (no randomness, so the
  // layout is stable across reloads). The simulation relaxes from here.
  var nodes = data.nodes.map(function (d, i) {{
    var a = 2 * Math.PI * i / n;
    return {{ slug: d.slug, title: d.title, x: CX + 240 * Math.cos(a), y: CY + 240 * Math.sin(a), vx: 0, vy: 0, fixed: false }};
  }});
  // Null-prototype maps: keys come from note slugs/indices, so a slug like
  // `__proto__` must not collide with Object.prototype and corrupt lookups.
  var index = Object.create(null);
  nodes.forEach(function (nd, i) {{ index[nd.slug] = i; }});

  var edges = [];
  var deg = Object.create(null);
  (data.edges || []).forEach(function (e) {{
    var a = index[e.s], b = index[e.t];
    if (a === undefined || b === undefined || a === b) return;
    edges.push([a, b]);
    deg[a] = (deg[a] || 0) + 1;
    deg[b] = (deg[b] || 0) + 1;
  }});
  var neighbors = nodes.map(function () {{ return Object.create(null); }});
  edges.forEach(function (e) {{ neighbors[e[0]][e[1]] = 1; neighbors[e[1]][e[0]] = 1; }});

  // Build SVG: edges first so nodes paint on top. Keep element refs.
  var edgeEls = edges.map(function (e) {{
    var ln = document.createElementNS(SVGNS, 'line');
    ln.setAttribute('class', 'graph-edge');
    ln.dataset.a = e[0]; ln.dataset.b = e[1];
    viewport.appendChild(ln);
    return ln;
  }});
  var nodeEls = nodes.map(function (nd, i) {{
    var g = document.createElementNS(SVGNS, 'g');
    g.setAttribute('class', 'graph-node');
    g.dataset.i = i;
    var r = 6 + Math.min(10, (deg[i] || 0) * 1.5);
    var c = document.createElementNS(SVGNS, 'circle');
    c.setAttribute('r', r);
    g.appendChild(c);
    var t = document.createElementNS(SVGNS, 'text');
    t.setAttribute('class', 'graph-node-label');
    t.setAttribute('dy', -r - 4);
    t.textContent = nd.title;
    g.appendChild(t);
    var title = document.createElementNS(SVGNS, 'title');
    title.textContent = nd.title;
    g.appendChild(title);
    viewport.appendChild(g);
    return g;
  }});

  function paint() {{
    for (var i = 0; i < edges.length; i++) {{
      var a = nodes[edges[i][0]], b = nodes[edges[i][1]];
      edgeEls[i].setAttribute('x1', a.x); edgeEls[i].setAttribute('y1', a.y);
      edgeEls[i].setAttribute('x2', b.x); edgeEls[i].setAttribute('y2', b.y);
    }}
    for (var j = 0; j < nodes.length; j++) {{
      nodeEls[j].setAttribute('transform', 'translate(' + nodes[j].x + ',' + nodes[j].y + ')');
    }}
  }}

  // Force simulation: repulsion (all pairs), spring attraction (edges), gentle
  // gravity toward center. Cooled by alpha; capped tick budget so a 500-node
  // graph settles fast and then the loop stops.
  var alpha = 1, MAX_TICKS = 260, tick = 0;
  var k = Math.sqrt((W * H) / Math.max(1, n));
  function step() {{
    for (var i = 0; i < n; i++) {{
      var ni = nodes[i];
      for (var j = i + 1; j < n; j++) {{
        var nj = nodes[j];
        var dx = ni.x - nj.x, dy = ni.y - nj.y;
        var dist = Math.sqrt(dx * dx + dy * dy) || 0.01;
        var rep = (k * k) / dist * 0.03;
        var fx = (dx / dist) * rep, fy = (dy / dist) * rep;
        ni.vx += fx; ni.vy += fy; nj.vx -= fx; nj.vy -= fy;
      }}
    }}
    for (var e = 0; e < edges.length; e++) {{
      var a = nodes[edges[e][0]], b = nodes[edges[e][1]];
      var dx2 = b.x - a.x, dy2 = b.y - a.y;
      var d2 = Math.sqrt(dx2 * dx2 + dy2 * dy2) || 0.01;
      var spring = (d2 - k) * 0.012;
      var sx = (dx2 / d2) * spring, sy = (dy2 / d2) * spring;
      a.vx += sx; a.vy += sy; b.vx -= sx; b.vy -= sy;
    }}
    for (var g = 0; g < n; g++) {{
      var nd = nodes[g];
      if (nd.fixed) {{ nd.vx = 0; nd.vy = 0; continue; }}
      nd.vx += (CX - nd.x) * 0.002;
      nd.vy += (CY - nd.y) * 0.002;
      nd.x += nd.vx * alpha; nd.y += nd.vy * alpha;
      nd.vx *= 0.85; nd.vy *= 0.85;
    }}
    alpha *= 0.985;
  }}
  var raf;
  function loop() {{
    var iters = n > 200 ? 1 : 2;
    for (var s = 0; s < iters; s++) {{ step(); tick++; }}
    paint();
    if (tick < MAX_TICKS && alpha > 0.02) raf = requestAnimationFrame(loop);
  }}
  paint();
  raf = requestAnimationFrame(loop);

  function reheat() {{ if (alpha < 0.3) alpha = 0.3; if (tick >= MAX_TICKS) {{ tick = 0; }} cancelAnimationFrame(raf); raf = requestAnimationFrame(loop); }}

  // Pan / zoom via a transform on the viewport group.
  var tx = 0, ty = 0, scale = 1;
  function applyView() {{ viewport.setAttribute('transform', 'translate(' + tx + ',' + ty + ') scale(' + scale + ')'); }}
  function toLocal(evt) {{
    var rect = svg.getBoundingClientRect();
    return {{ x: (evt.clientX - rect.left) / rect.width * W, y: (evt.clientY - rect.top) / rect.height * H }};
  }}

  var dragNode = null, panning = false, last = null, moved = false, downPt = null;
  svg.addEventListener('pointerdown', function (evt) {{
    var g = evt.target.closest ? evt.target.closest('.graph-node') : null;
    moved = false; downPt = toLocal(evt);
    if (g) {{
      dragNode = nodes[+g.dataset.i]; dragNode.fixed = true;
    }} else {{
      panning = true; last = {{ x: evt.clientX, y: evt.clientY }};
    }}
    svg.setPointerCapture(evt.pointerId);
  }});
  svg.addEventListener('pointermove', function (evt) {{
    if (dragNode) {{
      var p = toLocal(evt);
      dragNode.x = (p.x - tx) / scale; dragNode.y = (p.y - ty) / scale;
      moved = true; reheat(); paint();
    }} else if (panning && last) {{
      tx += evt.clientX - last.x; ty += evt.clientY - last.y;
      last = {{ x: evt.clientX, y: evt.clientY }}; moved = true; applyView();
    }}
  }});
  function endPointer(evt) {{
    var g = evt.target.closest ? evt.target.closest('.graph-node') : null;
    if (dragNode && !moved && g) {{ window.location.assign('/' + encodeURIComponent(nodes[+g.dataset.i].slug)); }}
    if (dragNode) dragNode.fixed = false;
    dragNode = null; panning = false; last = null;
  }}
  svg.addEventListener('pointerup', endPointer);
  svg.addEventListener('wheel', function (evt) {{
    evt.preventDefault();
    var p = toLocal(evt);
    var factor = evt.deltaY < 0 ? 1.1 : 0.9;
    var ns = Math.max(0.2, Math.min(4, scale * factor));
    tx = p.x - (p.x - tx) * (ns / scale); ty = p.y - (p.y - ty) * (ns / scale);
    scale = ns; applyView();
  }}, {{ passive: false }});

  // Hover: dim everything except the node and its neighbors / incident edges.
  function setHighlight(i) {{
    svg.classList.add('graph-hovering');
    nodeEls.forEach(function (g, j) {{
      var on = j === i || neighbors[i][j];
      g.classList.toggle('graph-node--hi', on);
      g.classList.toggle('graph-node--dim', !on);
    }});
    edgeEls.forEach(function (ln, e) {{
      var on = edges[e][0] === i || edges[e][1] === i;
      ln.classList.toggle('graph-edge--hi', on);
      ln.classList.toggle('graph-edge--dim', !on);
    }});
  }}
  function clearHighlight() {{
    svg.classList.remove('graph-hovering');
    nodeEls.forEach(function (g) {{ g.classList.remove('graph-node--hi', 'graph-node--dim'); }});
    edgeEls.forEach(function (ln) {{ ln.classList.remove('graph-edge--hi', 'graph-edge--dim'); }});
  }}
  nodeEls.forEach(function (g, i) {{
    g.addEventListener('pointerenter', function () {{ if (!dragNode && !panning) setHighlight(i); }});
    g.addEventListener('pointerleave', function () {{ if (!dragNode && !panning) clearHighlight(); }});
  }});
}})();
</script>"#
        )
    };

    render_page(
        site,
        HeadMeta {
            title: &format!("Graph \u{2014} {}", site.name),
            description: Some("An interactive map of every note and the links between them."),
            canonical_url: format!("{}/graph", site.base_url),
            og_type: "website",
            json_ld: None,
            csp_nonce: Some(csp_nonce),
            nav_current: Some("graph"),
            wide_layout: true,
        },
        &main,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::links::{Graph, GraphEdge, GraphNode};

    fn node(slug: &str, title: &str) -> GraphNode {
        GraphNode {
            slug: slug.to_string(),
            title: title.to_string(),
        }
    }

    fn edge(source: &str, target: &str) -> GraphEdge {
        GraphEdge {
            source_slug: source.to_string(),
            target_slug: target.to_string(),
        }
    }

    #[test]
    fn empty_graph_renders_a_message_without_canvas_or_script() {
        let site = SiteMeta::defaults();
        let html = render_graph_page(&Graph::default(), "nonce123", &site);

        assert!(html.contains(r#"<p class="empty">No published notes to graph yet.</p>"#));
        // Nav icons are inline <svg>, so scope the assertions to graph internals.
        assert!(!html.contains("graph-svg"));
        assert!(!html.contains("graph-canvas"));
        assert!(!html.contains("graph-data"));
    }

    #[test]
    fn non_empty_graph_renders_data_island_fallback_and_behavior_script() {
        let site = SiteMeta::defaults();
        let graph = Graph {
            nodes: vec![node("a", "Alpha"), node("b", "Beta")],
            edges: vec![edge("a", "b")],
        };
        let html = render_graph_page(&graph, "nonce123", &site);

        // Wide layout for the canvas.
        assert!(html.contains(r#"<main class="site-main wide-layout">"#));
        // Data island as JSON (not executable JS).
        assert!(
            html.contains(r#"<script type="application/json" id="graph-data" nonce="nonce123">"#)
        );
        // No-JS fallback links every node.
        assert!(html.contains(r#"<ul class="graph-fallback">"#));
        assert!(html.contains(r#"<li><a href="/a">Alpha</a></li>"#));
        assert!(html.contains(r#"<li><a href="/b">Beta</a></li>"#));
        // Canvas + behavior script.
        assert!(html.contains(r#"<svg class="graph-svg""#));
        assert!(html.contains(r#"<script nonce="nonce123">"#));
        assert!(html.contains("js-graph-active"));
    }

    #[test]
    fn data_island_escapes_a_hostile_title_so_it_cannot_break_out() {
        let site = SiteMeta::defaults();
        let graph = Graph {
            nodes: vec![node("x", "</script><x>")],
            edges: vec![],
        };
        let html = render_graph_page(&graph, "nonce123", &site);

        // The data island tag is present, but the hostile title's escaped form
        // cannot close the <script> island — the raw breakout never appears.
        assert!(html.contains(r#"id="graph-data""#));
        assert!(!html.contains("</script><x>"));
        // The literal angle brackets from the title are not emitted raw anywhere.
        assert!(!html.contains("<x>"));
    }

    #[test]
    fn the_behavior_script_carries_the_html_escaped_nonce() {
        let site = SiteMeta::defaults();
        let graph = Graph {
            nodes: vec![node("a", "Alpha")],
            edges: vec![],
        };
        let hostile = render_graph_page(&graph, r#""><x"#, &site);
        assert!(!hostile.contains(r#"<script nonce=""><x">"#));
        assert!(hostile.contains("&quot;&gt;&lt;x"));
    }
}
