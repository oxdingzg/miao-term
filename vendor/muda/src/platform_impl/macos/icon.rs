// Copyright 2022-2022 Tauri Programme within The Commons Conservancy
// SPDX-License-Identifier: Apache-2.0
// SPDX-License-Identifier: MIT

use objc2::{rc::Retained, ClassType};
use objc2_app_kit::NSImage;
use objc2_foundation::{CGFloat, NSData, NSSize};

use crate::icon::{BadIcon, RgbaIcon};
use std::io::Cursor;

#[derive(Debug, Clone)]
pub struct PlatformIcon(RgbaIcon);

impl PlatformIcon {
    pub fn from_rgba(rgba: Vec<u8>, width: u32, height: u32) -> Result<Self, BadIcon> {
        Ok(PlatformIcon(RgbaIcon::from_rgba(rgba, width, height)?))
    }

    pub fn get_size(&self) -> (u32, u32) {
        (self.0.width, self.0.height)
    }

    pub fn to_png(&self) -> Vec<u8> {
        let mut png = Vec::new();

        // miao patch: a zero-sized icon — what a process without an application
        // icon produces — used to panic here (`png` rejects a zero-width image
        // and the `unwrap` aborted). That panic runs inside an AppKit menu
        // callback, which cannot unwind, so clicking the standard About item
        // killed the whole app. Encode a transparent image of at least 1x1.
        let width = self.0.width.max(1) as usize;
        let height = self.0.height.max(1) as usize;
        let mut rgba = self.0.rgba.clone();
        rgba.resize(width * height * 4, 0);

        {
            let mut encoder = png::Encoder::new(Cursor::new(&mut png), width as _, height as _);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);

            let Ok(mut writer) = encoder.write_header() else {
                return Vec::new();
            };
            if writer.write_image_data(&rgba).is_err() {
                return Vec::new();
            }
        }

        png
    }

    pub fn to_nsimage(&self, fixed_height: Option<f64>) -> Retained<NSImage> {
        let (width, height) = self.get_size();
        let icon = self.to_png();

        let (icon_width, icon_height) = match fixed_height {
            Some(fixed_height) => {
                let icon_height: CGFloat = fixed_height as CGFloat;
                // miao patch: a zero-size icon used to divide by zero here.
                let icon_width: CGFloat =
                    (width.max(1) as CGFloat) / (height.max(1) as CGFloat / icon_height);

                (icon_width, icon_height)
            }

            None => (width.max(1) as CGFloat, height.max(1) as CGFloat),
        };

        let nsdata = NSData::with_bytes(&icon);

        // miao patch: an image AppKit cannot decode must not take the app down.
        let nsimage = match NSImage::initWithData(NSImage::alloc(), &nsdata) {
            Some(image) => image,
            None => unsafe { NSImage::init(NSImage::alloc()) },
        };
        let new_size = NSSize::new(icon_width, icon_height);
        unsafe { nsimage.setSize(new_size) };

        nsimage
    }
}
