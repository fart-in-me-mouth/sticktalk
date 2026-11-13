use iced::widget::image::Handle;
use std::{cell::RefCell, collections::VecDeque};
type Entry = (iced::advanced::image::Id, u32, Handle);
thread_local! {static MASKS:RefCell<VecDeque<Entry>>=const{RefCell::new(VecDeque::new())};}
pub(super) fn clear() {
    MASKS.with(|v| v.borrow_mut().clear());
}
pub fn masked(handle: &Handle, size: f32) -> Handle {
    let key = size.to_bits();
    MASKS.with(|cache| {
        let mut cache = cache.borrow_mut();
        if let Some((_, _, h)) = cache
            .iter()
            .find(|(id, s, _)| *id == handle.id() && *s == key)
        {
            return h.clone();
        }
        let Handle::Rgba {
            width,
            height,
            pixels,
            ..
        } = handle
        else {
            return handle.clone();
        };
        let Some(source) = image::RgbaImage::from_raw(*width, *height, pixels.to_vec()) else {
            return handle.clone();
        };
        let edge = (size * 2.).round().clamp(32., 160.) as u32;
        let side = (*width).min(*height);
        if side == 0 {
            return handle.clone();
        }
        let source = image::imageops::crop_imm(
            &source,
            (*width - side) / 2,
            (*height - side) / 2,
            side,
            side,
        )
        .to_image();
        let mut out =
            image::imageops::resize(&source, edge, edge, image::imageops::FilterType::Triangle);
        let ring = if size >= 80. { 5. } else { 3. };
        let dot = if size >= 80. { 16. } else { 10. };
        let radius = (dot / 2. + ring) / size * edge as f32;
        let center = edge as f32 - radius;
        for (x, y, p) in out.enumerate_pixels_mut() {
            let distance =
                ((x as f32 + 0.5 - center).powi(2) + (y as f32 + 0.5 - center).powi(2)).sqrt();
            let coverage = (distance - radius + 0.5).clamp(0., 1.);
            p.0[3] = (p.0[3] as f32 * coverage) as u8;
        }
        let result = Handle::from_rgba(edge, edge, out.into_raw());
        cache.push_back((handle.id(), key, result.clone()));
        while cache.len() > 64 {
            cache.pop_front();
        }
        result
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn status_notch_is_transparent_not_a_painted_dot() {
        let h = Handle::from_rgba(32, 32, vec![255; 32 * 32 * 4]);
        let Handle::Rgba { width, pixels, .. } = masked(&h, 32.) else {
            panic!()
        };
        let p = |x: usize, y: usize| pixels[(y * width as usize + x) * 4 + 3];
        assert_eq!(p(48, 48), 0);
        assert_eq!(p(20, 20), 255);
    }
}
