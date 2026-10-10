//! Treemapa „squarified” (Bruls, Huizing, van Wijk 2000) liczona po stronie Rusta.
//!
//! Wynik to płaska lista prostokątów zakodowana binarnie (24 B/prostokąt), żeby
//! przez IPC nie przechodziło całe drzewo ani ciężki JSON.

use crate::model::{Kind, Tree, NONE};
use crate::query::View;
use std::collections::VecDeque;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl Rect {
    pub fn area(&self) -> f64 {
        self.w * self.h
    }
}

pub const KIND_FILE: u8 = 0;
pub const KIND_DIR: u8 = 1;
/// Zagregowane drobne elementy („inne”); `id` wskazuje folder-rodzica.
pub const KIND_REST: u8 = 2;

#[derive(Clone, Copy, Debug)]
pub struct Tile {
    pub id: u32,
    pub rect: Rect,
    pub depth: u8,
    pub kind: u8,
    pub cat: u8,
    pub age: u8,
}

pub struct Options {
    pub width: f64,
    pub height: f64,
    /// Elementy o polu mniejszym niż to (px²) idą do „inne”.
    pub min_area: f64,
    pub max_tiles: usize,
    /// Odstęp wewnątrz folderu, żeby była widoczna struktura.
    pub padding: f64,
    pub now: i64,
}

/// Rozkłada `sizes` (posortowane malejąco, > 0) w prostokącie. Zwraca prostokąty w tej samej kolejności.
pub fn squarify(sizes: &[f64], rect: Rect) -> Vec<Rect> {
    let mut out = Vec::with_capacity(sizes.len());
    let total: f64 = sizes.iter().sum();
    if total <= 0.0 || rect.w <= 0.0 || rect.h <= 0.0 {
        return sizes.iter().map(|_| Rect { x: rect.x, y: rect.y, w: 0.0, h: 0.0 }).collect();
    }
    let scale = rect.area() / total;
    let areas: Vec<f64> = sizes.iter().map(|s| s * scale).collect();
    let mut r = rect;
    let mut i = 0;
    while i < areas.len() {
        let short = r.w.min(r.h);
        let mut j = i + 1;
        let mut sum = areas[i];
        let mut worst = worst_ratio(areas[i], areas[i], sum, short);
        while j < areas.len() {
            let nsum = sum + areas[j];
            // Lista jest malejąca, więc max = pierwszy, min = nowy.
            let nworst = worst_ratio(areas[i], areas[j], nsum, short);
            if nworst > worst {
                break;
            }
            worst = nworst;
            sum = nsum;
            j += 1;
        }
        let last_row = j == areas.len();
        if r.w >= r.h {
            // Kolumna przy lewej krawędzi.
            let cw = if last_row { r.w } else { sum / r.h };
            let mut y = r.y;
            for (k, a) in areas[i..j].iter().enumerate() {
                let h = if k + i + 1 == j { r.y + r.h - y } else { a / cw };
                out.push(Rect { x: r.x, y, w: cw, h });
                y += h;
            }
            r.x += cw;
            r.w -= cw;
        } else {
            // Wiersz przy górnej krawędzi.
            let rh = if last_row { r.h } else { sum / r.w };
            let mut x = r.x;
            for (k, a) in areas[i..j].iter().enumerate() {
                let w = if k + i + 1 == j { r.x + r.w - x } else { a / rh };
                out.push(Rect { x, y: r.y, w, h: rh });
                x += w;
            }
            r.y += rh;
            r.h -= rh;
        }
        i = j;
    }
    out
}

fn worst_ratio(max: f64, min: f64, sum: f64, side: f64) -> f64 {
    let s2 = sum * sum;
    let w2 = side * side;
    (w2 * max / s2).max(s2 / (w2 * min))
}

pub fn age_bucket(mtime: i64, now: i64) -> u8 {
    let d = (now - mtime).max(0) / 86_400;
    match d {
        0..=6 => 0,
        7..=30 => 1,
        31..=182 => 2,
        183..=365 => 3,
        366..=1095 => 4,
        _ => 5,
    }
}

fn is_container(tree: &Tree, id: u32, root: u32) -> bool {
    let n = tree.node(id);
    match n.kind {
        Kind::Dir => n.first_child != NONE,
        // Pakiet to jeden kafel, chyba że użytkownik wszedł do środka (jest korzeniem widoku).
        Kind::Package => id == root && n.first_child != NONE,
        _ => false,
    }
}

/// Układa treemapę poddrzewa `root`. Kolejność BFS: budżet kafli rozkłada się równo po poziomach.
pub fn layout(view: &View, root: u32, opts: &Options) -> Vec<Tile> {
    let tree = view.tree;
    let mut tiles = Vec::new();
    let mut queue = VecDeque::new();
    queue.push_back((root, Rect { x: 0.0, y: 0.0, w: opts.width, h: opts.height }, 0u8));

    while let Some((id, rect, depth)) = queue.pop_front() {
        if !is_container(tree, id, root) || view.size(id) == 0 {
            continue;
        }
        let mut kids: Vec<(u32, u64)> = view
            .visible_children(id)
            .into_iter()
            .map(|c| (c, view.size(c)))
            .filter(|&(_, s)| s > 0)
            .collect();
        kids.sort_unstable_by_key(|k| std::cmp::Reverse(k.1));
        // Własne bloki folderu (bez dzieci) nie dostają kafla – liczymy pole wg sumy dzieci.
        let kids_total: f64 = kids.iter().map(|k| k.1 as f64).sum();
        if kids_total <= 0.0 {
            continue;
        }
        let px_per_byte = rect.area() / kids_total;
        let budget_left = opts.max_tiles.saturating_sub(tiles.len());
        let mut keep = 0;
        for (k, &(_, s)) in kids.iter().enumerate() {
            if s as f64 * px_per_byte < opts.min_area || k + 1 >= budget_left.max(1) {
                break;
            }
            keep += 1;
        }
        let rest: u64 = kids[keep..].iter().map(|k| k.1).sum();
        let mut sizes: Vec<f64> = kids[..keep].iter().map(|k| k.1 as f64).collect();
        if rest > 0 {
            sizes.push(rest as f64);
        }
        if sizes.is_empty() {
            continue;
        }
        // „Inne” jest na końcu, ale squarify wymaga kolejności malejącej – sortujemy indeksy.
        let mut order: Vec<usize> = (0..sizes.len()).collect();
        order.sort_by(|&a, &b| sizes[b].total_cmp(&sizes[a]));
        let sorted: Vec<f64> = order.iter().map(|&i| sizes[i]).collect();
        let rects = squarify(&sorted, rect);

        for (pos, &i) in order.iter().enumerate() {
            let r = rects[pos];
            if i == keep {
                tiles.push(Tile { id, rect: r, depth: depth + 1, kind: KIND_REST, cat: 0, age: 0 });
                continue;
            }
            let (cid, _) = kids[i];
            let n = tree.node(cid);
            let container = is_container(tree, cid, root);
            tiles.push(Tile {
                id: cid,
                rect: r,
                depth: depth + 1,
                kind: if container { KIND_DIR } else { KIND_FILE },
                cat: n.cat,
                age: age_bucket(n.mtime, opts.now),
            });
            let p = opts.padding;
            if container && r.w > 2.0 * p + 2.0 && r.h > 2.0 * p + 2.0 && tiles.len() < opts.max_tiles {
                let inner = Rect { x: r.x + p, y: r.y + p, w: r.w - 2.0 * p, h: r.h - 2.0 * p };
                queue.push_back((cid, inner, depth.saturating_add(1)));
            }
        }
    }
    tiles
}

/// Kodowanie: [id u32][x f32][y f32][w f32][h f32][depth u8][kind u8][cat u8][age u8], little-endian.
pub fn encode(tiles: &[Tile]) -> Vec<u8> {
    let mut out = Vec::with_capacity(tiles.len() * 24);
    for t in tiles {
        out.extend_from_slice(&t.id.to_le_bytes());
        out.extend_from_slice(&(t.rect.x as f32).to_le_bytes());
        out.extend_from_slice(&(t.rect.y as f32).to_le_bytes());
        out.extend_from_slice(&(t.rect.w as f32).to_le_bytes());
        out.extend_from_slice(&(t.rect.h as f32).to_le_bytes());
        out.extend_from_slice(&[t.depth, t.kind, t.cat, t.age]);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn overlap(a: &Rect, b: &Rect) -> f64 {
        let w = (a.x + a.w).min(b.x + b.w) - a.x.max(b.x);
        let h = (a.y + a.h).min(b.y + b.h) - a.y.max(b.y);
        if w > 0.0 && h > 0.0 {
            w * h
        } else {
            0.0
        }
    }

    proptest! {
        #[test]
        fn squarify_is_proportional_and_disjoint(
            mut sizes in prop::collection::vec(1u32..1_000_000, 1..60),
            w in 50.0f64..2000.0,
            h in 50.0f64..2000.0,
        ) {
            sizes.sort_unstable_by(|a, b| b.cmp(a));
            let sizes: Vec<f64> = sizes.into_iter().map(f64::from).collect();
            let rect = Rect { x: 10.0, y: 20.0, w, h };
            let out = squarify(&sizes, rect);
            prop_assert_eq!(out.len(), sizes.len());
            let total: f64 = sizes.iter().sum();
            let area = rect.area();
            let eps = 1e-6 * area;
            let mut sum_area = 0.0;
            for (r, s) in out.iter().zip(&sizes) {
                prop_assert!(r.w >= -1e-9 && r.h >= -1e-9);
                prop_assert!(r.x >= rect.x - 1e-6 && r.y >= rect.y - 1e-6);
                prop_assert!(r.x + r.w <= rect.x + rect.w + 1e-6);
                prop_assert!(r.y + r.h <= rect.y + rect.h + 1e-6);
                let expected = s / total * area;
                prop_assert!((r.area() - expected).abs() <= expected * 1e-6 + 1e-6,
                    "pole {} ≠ oczekiwane {}", r.area(), expected);
                sum_area += r.area();
            }
            prop_assert!((sum_area - area).abs() <= eps + 1e-6);
            for i in 0..out.len() {
                for j in i + 1..out.len() {
                    prop_assert!(overlap(&out[i], &out[j]) <= 1e-6 * area + 1e-6);
                }
            }
        }
    }

    #[test]
    fn squares_for_equal_sizes() {
        let out = squarify(&[1.0, 1.0, 1.0, 1.0], Rect { x: 0.0, y: 0.0, w: 100.0, h: 100.0 });
        for r in out {
            assert!((r.w - 50.0).abs() < 1e-9 && (r.h - 50.0).abs() < 1e-9);
        }
    }

    #[test]
    fn encode_is_24_bytes_per_tile() {
        let t = Tile { id: 7, rect: Rect { x: 1.0, y: 2.0, w: 3.0, h: 4.0 }, depth: 1, kind: KIND_DIR, cat: 8, age: 5 };
        let b = encode(&[t, t]);
        assert_eq!(b.len(), 48);
        assert_eq!(u32::from_le_bytes(b[24..28].try_into().unwrap()), 7);
        assert_eq!(&b[20..24], &[1, KIND_DIR, 8, 5]);
    }

    #[test]
    fn age_buckets() {
        assert_eq!(age_bucket(0, 3 * 86_400), 0);
        assert_eq!(age_bucket(0, 400 * 86_400), 4);
        assert_eq!(age_bucket(0, 5000 * 86_400), 5);
    }
}
