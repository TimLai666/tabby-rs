//! Coordinate compatibility with Electron 38.8.6 / Chromium 140.0.7339.249.
//! The native adapter supplies monitors in EnumDisplayMonitors order, including
//! the exact Chromium display ID and float32 effective scale.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct Monitor {
    pub id: u32,
    pub physical: Rect,
    pub scale: f32,
}

#[derive(Debug)]
pub(crate) struct WindowsCoordinates {
    monitors: Vec<Monitor>,
    logical: Vec<Rect>,
    primary: usize,
}

fn edge(pos: i32, len: i32) -> i32 {
    pos.saturating_add(len)
}

fn sat_floor(value: f32) -> i32 {
    if value.is_nan() {
        0
    } else if value <= i32::MIN as f32 {
        i32::MIN
    } else if value >= i32::MAX as f32 {
        i32::MAX
    } else {
        value.floor() as i32
    }
}
fn sat_ceil(value: f32) -> i32 {
    if value.is_nan() {
        0
    } else if value <= i32::MIN as f32 {
        i32::MIN
    } else if value >= i32::MAX as f32 {
        i32::MAX
    } else {
        value.ceil() as i32
    }
}
fn rect_from_edges(x: i32, y: i32, right: i32, bottom: i32) -> Rect {
    Rect {
        x,
        y,
        width: right.saturating_sub(x).max(0),
        height: bottom.saturating_sub(y).max(0),
    }
}
fn scale_enclosing(r: Rect, scale: f32) -> Rect {
    if scale == 1.0 {
        return r;
    }
    let x = sat_floor(r.x as f32 * scale);
    let y = sat_floor(r.y as f32 * scale);
    let right = if r.width == 0 {
        x
    } else {
        sat_ceil(edge(r.x, r.width) as f32 * scale)
    };
    let bottom = if r.height == 0 {
        y
    } else {
        sat_ceil(edge(r.y, r.height) as f32 * scale)
    };
    rect_from_edges(x, y, right, bottom)
}
fn touches(a: Rect, b: Rect) -> bool {
    let l = a.x.max(b.x);
    let t = a.y.max(b.y);
    let r = edge(a.x, a.width).min(edge(b.x, b.width));
    let bot = edge(a.y, a.height).min(edge(b.y, b.height));
    (l == r && a.y <= edge(b.y, b.height) && b.y <= edge(a.y, a.height))
        || (t == bot && a.x <= edge(b.x, b.width) && b.x <= edge(a.x, a.width))
}
fn scale_offset(length: i32, scale: f32, offset: i32) -> i32 {
    let scaled_length = length as f32 / scale;
    let percent = offset as f32 / length as f32;
    sat_floor(scaled_length * percent)
}
#[derive(Clone, Copy)]
enum Pos {
    Top,
    Right,
    Bottom,
    Left,
}
#[derive(Clone, Copy)]
struct Placement {
    parent: usize,
    child: usize,
    pos: Pos,
    offset: i32,
    end: bool,
}

fn placement(monitors: &[Monitor], parent: usize, child: usize) -> Placement {
    let a = monitors[parent].physical;
    let b = monitors[child].physical;
    let max_left = a.x.max(b.x);
    let max_top = a.y.max(b.y);
    let min_right = edge(a.x, a.width).min(edge(b.x, b.width));
    let min_bottom = edge(a.y, a.height).min(edge(b.y, b.height));
    let pos = if max_left == min_right && max_top == min_bottom {
        if edge(a.y, a.height) == max_top {
            Pos::Bottom
        } else if a.x == max_left {
            Pos::Left
        } else {
            Pos::Top
        }
    } else if max_left == min_right && a.y <= edge(b.y, b.height) && b.y <= edge(a.y, a.height) {
        if a.x == max_left {
            Pos::Left
        } else {
            Pos::Right
        }
    } else if a.y.max(b.y) == min_bottom && a.x <= edge(b.x, b.width) && b.x <= edge(a.x, a.width) {
        if a.y == max_top {
            Pos::Top
        } else {
            Pos::Bottom
        }
    } else {
        unreachable!()
    };
    let (pb, pe, cb, ce) = match pos {
        Pos::Top | Pos::Bottom => (a.x, edge(a.x, a.width), b.x, edge(b.x, b.width)),
        Pos::Left | Pos::Right => (a.y, edge(a.y, a.height), b.y, edge(b.y, b.height)),
    };
    let plen = pe - pb;
    let cb = cb - pb;
    let ce = ce - pb;
    let (offset, end) = if plen == ce {
        (0, true)
    } else if cb >= 0 && cb <= plen {
        (scale_offset(plen, monitors[parent].scale, cb), false)
    } else if ce >= 0 && ce <= plen {
        (scale_offset(plen, monitors[parent].scale, plen - ce), true)
    } else {
        (scale_offset(ce - cb, monitors[child].scale, cb), false)
    };
    Placement {
        parent,
        child,
        pos,
        offset,
        end,
    }
}
fn apply(p: Placement, logical: &mut [Rect]) {
    let par = logical[p.parent];
    let child = logical[p.child];
    let (plen, clen) = match p.pos {
        Pos::Top | Pos::Bottom => (par.width, child.width),
        _ => (par.height, child.height),
    };
    let mut off = if p.end {
        plen.saturating_sub(p.offset).saturating_sub(clen)
    } else {
        p.offset
    };
    off = off.clamp(-clen, plen);
    let (x, y) = match p.pos {
        Pos::Top => (
            par.x.saturating_add(off),
            par.y.saturating_sub(child.height),
        ),
        Pos::Right => (edge(par.x, par.width), par.y.saturating_add(off)),
        Pos::Bottom => (par.x.saturating_add(off), edge(par.y, par.height)),
        Pos::Left => (par.x.saturating_sub(child.width), par.y.saturating_add(off)),
    };
    logical[p.child].x = x;
    logical[p.child].y = y;
}
fn intersection(a: Rect, b: Rect) -> (i64, i64, i64) {
    let w = (edge(a.x, a.width).min(edge(b.x, b.width)) as i64 - a.x.max(b.x) as i64).max(0);
    let h = (edge(a.y, a.height).min(edge(b.y, b.height)) as i64 - a.y.max(b.y) as i64).max(0);
    (w, h, w * h)
}
fn rect_metric(reference: Rect, other: Rect) -> i128 {
    let (_, _, area) = intersection(reference, other);
    if area > 0 {
        return -area as i128;
    }

    // Chromium rotates the pair so the reference is above the other rect,
    // then measures either corner distance or the remaining vertical gap.
    let rotation = if edge(reference.y, reference.height) <= other.y {
        0
    } else if edge(other.x, other.width) <= reference.x {
        90
    } else if edge(other.y, other.height) <= reference.y {
        180
    } else {
        270
    };
    let rotate = |rect: Rect| -> (i64, i64, i64, i64) {
        let (x, y, w, h) = (
            rect.x as i64,
            rect.y as i64,
            rect.width as i64,
            rect.height as i64,
        );
        match rotation {
            0 => (x, y, w, h),
            90 => (y, -x - w, h, w),
            180 => (-x - w, -y - h, w, h),
            270 => (-y - h, x, h, w),
            _ => unreachable!(),
        }
    };
    let (tx, ty, tw, th) = rotate(reference);
    let (bx, by, bw, _bh) = rotate(other);
    let tr = tx + tw;
    let tb = ty + th;
    let br = bx + bw;
    let distance = if br < tx {
        let dx = br - tx;
        let dy = by - tb;
        dx as i128 * dx as i128 + dy as i128 * dy as i128
    } else if tr < bx {
        let dx = bx - tr;
        let dy = by - tb;
        dx as i128 * dx as i128 + dy as i128 * dy as i128
    } else {
        let dy = by - tb;
        dy as i128 * dy as i128
    };
    distance
}
fn origin_distance2(r: Rect) -> i128 {
    r.x as i128 * r.x as i128 + r.y as i128 * r.y as i128
}

impl WindowsCoordinates {
    pub fn new(monitors: Vec<Monitor>) -> Result<Self, &'static str> {
        if monitors.is_empty() {
            return Err("missing monitors");
        }
        for (i, m) in monitors.iter().enumerate() {
            if monitors[..i].iter().any(|prior| prior.id == m.id) {
                return Err("duplicate monitor id");
            }
            if !m.scale.is_finite() || m.scale <= 0.0 {
                return Err("invalid monitor scale");
            }
            if m.physical.width <= 0 || m.physical.height <= 0 {
                return Err("invalid monitor bounds");
            }
        }
        let primary = monitors
            .iter()
            .position(|m| m.physical.x == 0 && m.physical.y == 0)
            .ok_or("missing primary")?;
        let mut logical: Vec<Rect> = monitors
            .iter()
            .map(|m| scale_enclosing(m.physical, 1.0 / m.scale))
            .collect();
        logical[primary].x = 0;
        logical[primary].y = 0;
        let mut remaining: Vec<usize> = (0..monitors.len()).filter(|&i| i != primary).collect();
        let mut parents = vec![primary];
        let mut placements = Vec::new();
        let mut depth = vec![100usize; monitors.len()];
        depth[primary] = 0;
        while let Some(parent) = parents.pop() {
            let mut first = 0usize;
            let mut last = remaining.len();
            while first < last {
                while first < last
                    && !touches(
                        monitors[parent].physical,
                        monitors[remaining[first]].physical,
                    )
                {
                    first += 1;
                }
                while first < last
                    && touches(
                        monitors[parent].physical,
                        monitors[remaining[last - 1]].physical,
                    )
                {
                    last -= 1;
                }
                if first < last {
                    remaining.swap(first, last - 1);
                    first += 1;
                    last -= 1;
                }
            }
            let touching = remaining.split_off(first);
            for child in touching {
                placements.push(placement(&monitors, parent, child));
                depth[child] = (depth[parent] + 1).min(100);
                parents.push(child);
            }
        }
        for p in &placements {
            apply(*p, &mut logical);
        }
        if !placements.is_empty() && monitors.len() > 1 {
            let mut order: Vec<usize> = (0..monitors.len()).collect();
            order.sort_by_key(|&i| (depth[i], origin_distance2(logical[i]), monitors[i].id));
            for i in 1..order.len() {
                let target = order[i];
                for &source in &order[..i] {
                    let s = logical[source];
                    let t = logical[target];
                    let (w, h, area) = intersection(s, t);
                    if area == 0 {
                        continue;
                    }
                    let ox = if w > 0 {
                        if t.x >= 0 {
                            edge(s.x, s.width).saturating_sub(t.x)
                        } else {
                            edge(t.x, t.width).saturating_sub(s.x).saturating_neg()
                        }
                    } else {
                        0
                    };
                    let oy = if h > 0 {
                        if t.y >= 0 {
                            edge(s.y, s.height).saturating_sub(t.y)
                        } else {
                            edge(t.y, t.height).saturating_sub(s.y).saturating_neg()
                        }
                    } else {
                        0
                    };
                    let (dx, dy) = if ox.unsigned_abs() <= oy.unsigned_abs() {
                        (ox, 0)
                    } else {
                        (0, oy)
                    };
                    if dx != 0 || dy != 0 {
                        logical[target].x = logical[target].x.saturating_add(dx);
                        logical[target].y = logical[target].y.saturating_add(dy);
                    }
                }
            }
        }
        Ok(Self {
            monitors,
            logical,
            primary,
        })
    }
    pub fn logical_bounds(&self, monitor_id: u32) -> Option<Rect> {
        self.monitors
            .iter()
            .position(|m| m.id == monitor_id)
            .map(|i| self.logical[i])
    }
    pub fn physical_to_logical(&self, bounds: Rect, monitor_id: u32) -> Option<Rect> {
        let i = self.monitors.iter().position(|m| m.id == monitor_id)?;
        let m = self.monitors[i];
        let d = self.logical[i];
        let inv = 1.0f32 / m.scale;
        let x = sat_floor((bounds.x as f32 - m.physical.x as f32) * inv + d.x as f32);
        let y = sat_floor((bounds.y as f32 - m.physical.y as f32) * inv + d.y as f32);
        let scaled = scale_enclosing(bounds, inv);
        Some(Rect {
            x,
            y,
            width: scaled.width,
            height: scaled.height,
        })
    }
    pub fn logical_to_physical(&self, bounds: Rect) -> Rect {
        let mut best = self.primary;
        let mut best_metric = i128::MAX;
        for i in 0..self.monitors.len() {
            let metric = rect_metric(bounds, self.logical[i]);
            if metric < best_metric {
                best = i;
                best_metric = metric;
            }
        }
        let m = self.monitors[best];
        let d = self.logical[best];
        let s = m.scale;
        let x = sat_floor((bounds.x as f32 - d.x as f32) * s + m.physical.x as f32);
        let y = sat_floor((bounds.y as f32 - d.y as f32) * s + m.physical.y as f32);
        let scaled = scale_enclosing(bounds, s);
        Rect {
            x,
            y,
            width: scaled.width,
            height: scaled.height,
        }
    }
    pub fn logical_point_to_physical(&self, x: i32, y: i32) -> (i32, i32) {
        let i = (0..self.logical.len())
            .find(|&i| {
                x >= self.logical[i].x
                    && y >= self.logical[i].y
                    && x < edge(self.logical[i].x, self.logical[i].width)
                    && y < edge(self.logical[i].y, self.logical[i].height)
            })
            .unwrap_or(self.primary);
        let m = self.monitors[i];
        let d = self.logical[i];
        (
            sat_floor((x as f32 - d.x as f32) * m.scale + m.physical.x as f32),
            sat_floor((y as f32 - d.y as f32) * m.scale + m.physical.y as f32),
        )
    }
}

#[cfg(test)]
#[path = "window_geometry_windows_coordinates_tests.rs"]
mod tests;
