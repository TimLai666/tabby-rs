//! Locate the complete, fixed dark TooltipAura border independently of a candidate.
pub fn find(width: u32, height: u32, rgba: &[u8]) -> Result<[u32; 4], String> {
    if width == 0
        || height == 0
        || width > 4096
        || height > 4096
        || rgba.len() != width as usize * height as usize * 4
    {
        return Err("tooltip reference must be bounded RGBA8".into());
    }
    let pixel = |x: u32, y: u32| {
        let index = ((y * width + x) * 4) as usize;
        &rgba[index..index + 4]
    };
    let border = |x, y| pixel(x, y) == [202, 202, 202, 255];
    let mut found = None;
    for y in 0..height {
        let mut x = 0;
        while x < width {
            if !border(x, y) {
                x += 1;
                continue;
            }
            let left = x;
            while x < width && border(x, y) {
                x += 1;
            }
            let w = x - left;
            if !(18..=800).contains(&w) {
                continue;
            }
            let right = x - 1;
            for bottom in y.saturating_add(9)..height.min(y + 96) {
                if !(left..=right).all(|x| border(x, bottom))
                    || !(y..=bottom).all(|y| border(left, y) && border(right, y))
                    || pixel(left + 1, y + 1) != [31, 31, 31, 255]
                {
                    continue;
                }
                if found.is_some() {
                    return Err("ambiguous tooltip reference borders".into());
                }
                found = Some([left, y, w, bottom - y + 1]);
            }
        }
    }
    found.ok_or_else(|| "complete dark TooltipAura border is missing".into())
}

#[cfg(test)]
mod tests {
    use super::find;

    fn draw(image: &mut [u8], width: usize, rect: [usize; 4]) {
        let [left, top, w, h] = rect;
        for y in top..top + h {
            for x in left..left + w {
                let border = x == left || x == left + w - 1 || y == top || y == top + h - 1;
                let color = if border { 202 } else { 31 };
                image[(y * width + x) * 4..(y * width + x + 1) * 4]
                    .copy_from_slice(&[color, color, color, 255]);
            }
        }
    }

    #[test]
    fn complete_original_border_sets_the_crop_despite_text_pixels() {
        let mut image = vec![0; 100 * 70 * 4];
        draw(&mut image, 100, [15, 20, 60, 25]);
        image[(27 * 100 + 24) * 4..(27 * 100 + 25) * 4].copy_from_slice(&[202, 169, 127, 255]);
        assert_eq!(find(100, 70, &image).unwrap(), [15, 20, 60, 25]);
    }

    #[test]
    fn partial_or_translucent_borders_are_not_reference_tooltips() {
        let mut image = vec![0; 100 * 70 * 4];
        draw(&mut image, 100, [15, 20, 60, 25]);
        image[(30 * 100 + 15) * 4] = 201;
        assert!(find(100, 70, &image).is_err());
        draw(&mut image, 100, [15, 20, 60, 25]);
        image[(20 * 100 + 15) * 4 + 3] = 254;
        assert!(find(100, 70, &image).is_err());
    }

    #[test]
    fn ambiguous_rectangles_and_malformed_buffers_fail() {
        let mut image = vec![0; 100 * 70 * 4];
        draw(&mut image, 100, [2, 2, 30, 25]);
        draw(&mut image, 100, [50, 35, 40, 25]);
        assert!(find(100, 70, &image).is_err());
        assert!(find(100, 70, &image[..100]).is_err());
        assert!(find(0, 0, &[]).is_err());
        assert!(find(u32::MAX, u32::MAX, &[]).is_err());
    }

    #[test]
    fn a_popup_clipped_at_its_parent_right_edge_is_rejected() {
        let mut complete = vec![0; 61 * 25 * 4];
        draw(&mut complete, 61, [0, 0, 61, 25]);
        assert_eq!(find(61, 25, &complete).unwrap(), [0, 0, 61, 25]);
        let clipped = complete
            .chunks_exact(61 * 4)
            .flat_map(|row| row[..60 * 4].iter().copied())
            .collect::<Vec<_>>();
        assert!(find(60, 25, &clipped).is_err());
    }

    #[test]
    fn background_and_filled_rectangles_are_not_tooltips() {
        let image = vec![202; 100 * 70 * 4];
        assert!(find(100, 70, &image).is_err());
    }
}
