//! Reads the text lines of an image on this machine: a text detection model finds where the lines are, a recognition
//! model reads each one. The models are PaddleOCR's (PP-OCRv5 mobile detection, Cyrillic recognition) as ONNX files,
//! run by `tract` in pure Rust, so nothing leaves the computer and no native library ships with the app.
//!
//! The steps and their numbers follow the models' reference pipeline (RapidOCR): what the models were tuned with.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use image::{imageops, ImageDecoder, RgbImage};
use tract_onnx::prelude::*;

use super::{Line, Page};

pub const DETECTION_MODEL: &str = "ch_PP-OCRv5_det_mobile.onnx";
pub const RECOGNITION_MODEL: &str = "cyrillic_PP-OCRv5_rec_mobile.onnx";

/// How the pipeline runs. A document config set carries these: the engine has no values of its own.
#[derive(serde::Deserialize, Clone, Debug, PartialEq)]
pub struct OcrSettings {
    /// an image is scaled so its shorter side is at least this before detection, and its longer side at most
    /// `max_side`
    pub min_side: f64,
    pub max_side: f64,
    /// a pixel is text above this; a box is kept when its mean is above `box_score`
    pub text: f32,
    pub box_score: f32,
    /// how far a found box grows to take in the whole line
    pub unclip: f64,
    /// a line read with less confidence than this is left out
    pub min_confidence: f32,
    /// sideways and upside-down pictures are stood upright
    pub turn: bool,
    /// rounds of looking again, larger, at the part of the picture around the text found
    pub zoom: usize,
    /// a picture that leans is laid level by the lean of its long lines
    pub level: bool,
    /// words on one row closer than this many line heights are one line
    pub join_gap: f64,
}

const MAX_BOXES: usize = 1000;
/// Recognition takes lines of this height, in widths of WIDTH_STEP between MIN_WIDTH and MAX_WIDTH.
const LINE_HEIGHT: u32 = 48;
const WIDTH_STEP: u32 = 160;
const MIN_WIDTH: u32 = 320;
const MAX_WIDTH: u32 = 2560;
/// A lean under half a degree is left alone.
const MIN_SKEW: f64 = 0.0087;

type Plan = Arc<TypedRunnableModel>;

/// A text line's place in the image: its centre, the unit vector along the text, and half its length and height.
#[derive(Clone, Copy)]
struct TextBox {
    centre: (f64, f64),
    along: (f64, f64),
    half: (f64, f64),
}

impl TextBox {
    /// The angle the text runs at, in radians; 0 is level.
    fn lean(&self) -> f64 {
        self.along.1.atan2(self.along.0)
    }

    fn across(&self) -> (f64, f64) {
        (-self.along.1, self.along.0)
    }

    /// Top-left, top-right, bottom-right, bottom-left.
    fn corners(&self) -> [(f64, f64); 4] {
        let (c, u, v, (a, b)) = (self.centre, self.along, self.across(), self.half);
        let at = |su: f64, sv: f64| (c.0 + su * a * u.0 + sv * b * v.0, c.1 + su * a * u.1 + sv * b * v.1);
        [at(-1.0, -1.0), at(1.0, -1.0), at(1.0, 1.0), at(-1.0, 1.0)]
    }
}

/// The two models, loaded once; a runnable plan is made per input size and kept.
pub struct Engine {
    detection: InferenceModel,
    recognition: InferenceModel,
    /// what each output class of recognition is; class 0 is "nothing here"
    characters: Vec<String>,
    plans: Mutex<HashMap<(bool, usize, usize), Plan>>,
}

/// Where the model files are: the folder named by BLANKEY_OCR, beside the executable as the installers put them, or
/// `src-tauri/ocr` in a checkout (scripts/fetch-ocr.sh fills it).
pub fn model_dir() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok();
    let beside = |up: &str| exe.as_ref().and_then(|exe| Some(exe.parent()?.join(up)));
    let candidates = [
        std::env::var_os("BLANKEY_OCR").map(PathBuf::from),
        beside("ocr"),
        beside("../Resources/ocr"),
        beside("../lib/blankey/ocr"),
        Some(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../ocr")),
    ];
    candidates
        .into_iter()
        .flatten()
        .find(|dir| dir.join(DETECTION_MODEL).is_file() && dir.join(RECOGNITION_MODEL).is_file())
}

impl Engine {
    pub fn load(dir: &Path) -> Result<Engine, String> {
        Engine::load_model(dir, RECOGNITION_MODEL)
    }

    /// The engine with another recognition model of the folder: for trying a set against one the app does not ship.
    pub fn load_model(dir: &Path, recognition: &str) -> Result<Engine, String> {
        let text = |e: TractError| format!("OCR model: {e}");
        // the files name the sizes inside the network by symbols that clash with a fixed input size: left out, the
        // sizes are worked out from the input
        let onnx = tract_onnx::onnx().with_ignore_value_info(true);
        let detection = onnx.model_for_path(dir.join(DETECTION_MODEL)).map_err(text)?;
        let proto = onnx.proto_model_for_path(dir.join(recognition)).map_err(text)?;
        // the model file carries its own character list, one per line
        let listed =
            proto.metadata_props.iter().find(|entry| entry.key == "character").ok_or("OCR model: no character list")?;
        let mut characters: Vec<String> = listed.value.lines().map(str::to_string).collect();
        characters.push(" ".into());
        let recognition = onnx.model_for_proto_model(&proto).map_err(text)?;
        Ok(Engine { detection, recognition, characters, plans: Mutex::new(HashMap::new()) })
    }

    fn plan(&self, detect: bool, height: usize, width: usize) -> Result<Plan, String> {
        let key = (detect, height, width);
        if let Some(plan) = self.plans.lock().unwrap().get(&key) {
            return Ok(plan.clone());
        }
        let model = if detect { &self.detection } else { &self.recognition };
        let shaped = model.clone().with_input_fact(0, f32::fact([1, 3, height, width]).into());
        let plan = shaped
            .and_then(|m| m.into_optimized())
            .and_then(|m| m.into_runnable())
            .map_err(|e| format!("OCR model: {e}"))?;
        self.plans.lock().unwrap().insert(key, plan.clone());
        Ok(plan)
    }

    /// The model's output for one image, as (shape, values).
    fn run(&self, detect: bool, image: &RgbImage) -> Result<(Vec<usize>, Vec<f32>), String> {
        let (width, height) = (image.width() as usize, image.height() as usize);
        // the models were trained on blue-green-red images scaled to -1..1
        let mut input = vec![0f32; 3 * height * width];
        for (x, y, pixel) in image.enumerate_pixels() {
            for channel in 0..3 {
                input[channel * height * width + y as usize * width + x as usize] =
                    pixel[2 - channel] as f32 / 127.5 - 1.0;
            }
        }
        let tensor = Tensor::from_shape(&[1, 3, height, width], &input).map_err(|e| e.to_string())?;
        let output = self.plan(detect, height, width)?.run(tvec!(tensor.into())).map_err(|e| format!("OCR: {e}"))?;
        let view = output[0].to_plain_array_view::<f32>().map_err(|e| e.to_string())?;
        Ok((view.shape().to_vec(), view.iter().copied().collect()))
    }

    /// The text lines of an image file's content (PNG, JPEG or WebP). Positions are in the pixels of the picture
    /// stood upright: as a viewer shows it, turned by the page's `turned` quarter turns.
    pub fn read(&self, content: &[u8], settings: &OcrSettings) -> Result<Page, String> {
        self.read_image(&decode(content)?, settings)
    }

    pub fn read_image(&self, image: &RgbImage, settings: &OcrSettings) -> Result<Page, String> {
        // a card photographed sideways: most of its lines run up the picture
        let (mut boxes, sideways) = self.detect(image, settings)?;
        let mut turned = 0u8;
        let mut upright = std::borrow::Cow::Borrowed(image);
        if settings.turn && sideways > boxes.len() {
            upright = std::borrow::Cow::Owned(imageops::rotate90(image));
            turned = 1;
            boxes = self.detect(&upright, settings)?.0;
        }
        let (mut lines, mut skew) = self.lines(&upright, &self.zoomed(&upright, boxes.clone(), settings)?, settings)?;
        // upside down, lines are found and read as nothing: the other way up decides
        if settings.turn && boxes.len() >= 4 && lines.len() * 2 < boxes.len() {
            let flipped = imageops::rotate180(upright.as_ref());
            let found = self.detect(&flipped, settings)?.0;
            let (other, other_skew) = self.lines(&flipped, &self.zoomed(&flipped, found, settings)?, settings)?;
            let read = |lines: &[Line]| lines.iter().map(|l| l.text.chars().count()).sum::<usize>();
            if read(&other) > read(&lines) {
                (lines, skew, turned) = (other, other_skew, turned + 2);
            }
        }
        let mut lines = join_words(lines, settings.join_gap);
        lines.sort_by(|a, b| {
            (a.rect[1], a.rect[0]).partial_cmp(&(b.rect[1], b.rect[0])).unwrap_or(std::cmp::Ordering::Equal)
        });
        Ok(Page { width: upright.width() as f64, height: upright.height() as f64, lines, turned, skew })
    }

    /// The boxes read into lines, and the lean of the picture they were laid level by: a line's place is where it
    /// would be in the picture turned level about its middle, so the lines of a row are on one row. A box that
    /// reads as nothing is left out.
    fn lines(&self, image: &RgbImage, boxes: &[TextBox], settings: &OcrSettings) -> Result<(Vec<Line>, f64), String> {
        let long = boxes.iter().filter(|found| found.half.0 > 3.0 * found.half.1);
        let skew = Some(middle_of(long.map(TextBox::lean).collect()))
            .filter(|lean| settings.level && lean.abs() > MIN_SKEW)
            .unwrap_or(0.0);
        let (cx, cy) = (image.width() as f64 / 2.0, image.height() as f64 / 2.0);
        let level = |(x, y): (f64, f64)| {
            let (dx, dy) = (x - cx, y - cy);
            (cx + dx * skew.cos() + dy * skew.sin(), cy - dx * skew.sin() + dy * skew.cos())
        };
        let mut lines = Vec::new();
        for found in boxes {
            let Some((text, confidence)) = self.recognize(&crop(image, found), settings.min_confidence)? else {
                continue;
            };
            let corners = found.corners().map(level);
            let (xs, ys) = (corners.map(|c| c.0), corners.map(|c| c.1));
            let low = |values: [f64; 4]| values.into_iter().fold(f64::MAX, f64::min).round();
            let high = |values: [f64; 4]| values.into_iter().fold(f64::MIN, f64::max).round();
            lines.push(Line { text, rect: [low(xs), low(ys), high(xs), high(ys)], confidence });
        }
        Ok((lines, skew))
    }

    /// A card that takes a small part of a photo has print too small to find at the size the whole photo is
    /// looked at: only its titles show. The part of the picture around what was found is looked at again, larger,
    /// until the text fills a fair share of what is looked at.
    fn zoomed(
        &self,
        image: &RgbImage,
        mut boxes: Vec<TextBox>,
        settings: &OcrSettings,
    ) -> Result<Vec<TextBox>, String> {
        let (width, height) = (image.width() as f64, image.height() as f64);
        let (mut left, mut top, mut right, mut bottom) = (0.0, 0.0, width, height);
        for _ in 0..settings.zoom {
            if boxes.len() < 2 {
                break;
            }
            let corners = boxes.iter().flat_map(|b| b.corners());
            let (x0, y0, x1, y1) = corners.fold((f64::MAX, f64::MAX, f64::MIN, f64::MIN), |r, (x, y)| {
                (r.0.min(x), r.1.min(y), r.2.max(x), r.3.max(y))
            });
            // room around it for the rest of the card: its titles span its width, and it is about two thirds as tall
            let (w, h) = (x1 - x0, y1 - y0);
            let grow = (0.8 * w - h).max(0.15 * h);
            let (x0, x1) = ((x0 - 0.15 * w).max(left), (x1 + 0.15 * w).min(right));
            let (y0, y1) = ((y0 - grow).max(top), (y1 + grow).min(bottom));
            if (x1 - x0) * (y1 - y0) > 0.5 * (right - left) * (bottom - top) {
                break;
            }
            let part =
                imageops::crop_imm(image, x0 as u32, y0 as u32, ((x1 - x0) as u32).max(1), ((y1 - y0) as u32).max(1))
                    .to_image();
            let mut closer = self.detect(&part, settings)?.0;
            if closer.len() < boxes.len() {
                break;
            }
            for found in &mut closer {
                found.centre = (found.centre.0 + x0.floor(), found.centre.1 + y0.floor());
            }
            (boxes, left, top, right, bottom) = (closer, x0, y0, x1, y1);
        }
        Ok(boxes)
    }

    /// Where the text lines are, in the image's pixels, and how many more run up or down the picture.
    fn detect(&self, image: &RgbImage, settings: &OcrSettings) -> Result<(Vec<TextBox>, usize), String> {
        let (width, height) = (image.width() as f64, image.height() as f64);
        let mut scale = (settings.max_side / width.max(height)).min(1.0);
        if width.min(height) * scale < settings.min_side {
            scale = settings.min_side / width.min(height);
        }
        // the network halves its input five times
        let side = |length: f64| (((length * scale / 32.0).round() as u32).max(1)) * 32;
        let (w, h) = (side(width), side(height));
        let resized = imageops::resize(image, w, h, imageops::FilterType::Triangle);
        let (_, map) = self.run(true, &resized)?;
        let (w, h) = (w as usize, h as usize);

        // text pixels, grown by one so the letters of a word join
        let text = |x: usize, y: usize| map[y * w + x] > settings.text;
        let mut mask = vec![false; w * h];
        for y in 0..h {
            for x in 0..w {
                mask[y * w + x] = text(x, y)
                    || (x > 0 && text(x - 1, y))
                    || (y > 0 && text(x, y - 1))
                    || (x > 0 && y > 0 && text(x - 1, y - 1));
            }
        }
        let regions: Vec<_> = regions(&mut mask, w, h).into_iter().take(MAX_BOXES).collect();
        // a short word cannot tell which way it leans: the long lines of the picture say
        let lean = middle_of(
            regions
                .iter()
                .filter_map(|region| bounding(region, 0.0))
                .filter(|(_, own)| *own)
                .map(|(found, _)| found.lean())
                .collect(),
        );
        let (mut boxes, mut sideways) = (Vec::new(), 0);
        for region in regions {
            let Some((mut found, _)) = bounding(&region, lean) else { continue };
            if found.half.0.min(found.half.1) * 2.0 < 3.0 {
                continue;
            }
            let score = region.iter().map(|&(x, y)| map[y * w + x]).sum::<f32>() / region.len() as f32;
            if score < settings.box_score {
                continue;
            }
            let (a, b) = found.half;
            let grow = settings.unclip * a * b / (a + b);
            found.half = (a + grow, b + grow);
            if found.half.1 * 2.0 < 5.0 {
                continue;
            }
            // back to the image's own pixels
            let (sx, sy) = (width / w as f64, height / h as f64);
            found.centre = (found.centre.0 * sx, found.centre.1 * sy);
            found.half = (found.half.0 * sx, found.half.1 * sy);
            // text that runs up the side of a card is decoration, unless most of the picture's text does
            if found.half.1 < 1.5 * found.half.0 {
                boxes.push(found);
            } else if found.half.1 > 2.0 * found.half.0 {
                sideways += 1;
            }
        }
        Ok((boxes, sideways))
    }

    /// The text of one cropped line and how sure the model is; None for a line that reads as nothing.
    fn recognize(&self, line: &RgbImage, min_confidence: f32) -> Result<Option<(String, f32)>, String> {
        let wide = (LINE_HEIGHT as f64 * line.width() as f64 / line.height().max(1) as f64).ceil() as u32;
        let wide = wide.clamp(1, MAX_WIDTH);
        let resized = imageops::resize(line, wide, LINE_HEIGHT, imageops::FilterType::Triangle);
        // the line sits on the left of an empty canvas; a few canvas widths keep the number of plans small
        let canvas_width = (wide.div_ceil(WIDTH_STEP) * WIDTH_STEP).max(MIN_WIDTH);
        let mut canvas = RgbImage::from_pixel(canvas_width, LINE_HEIGHT, image::Rgb([127, 127, 127]));
        imageops::overlay(&mut canvas, &resized, 0, 0);
        let (shape, output) = self.run(false, &canvas)?;
        let (steps, classes) = (shape[1], shape[2]);

        // one class per step; a class repeated over steps is one character, class 0 separates them
        let (mut text, mut sure, mut kept, mut last) = (String::new(), 0f32, 0usize, 0usize);
        for step in 0..steps {
            let scores = &output[step * classes..(step + 1) * classes];
            let (class, score) =
                scores
                    .iter()
                    .copied()
                    .enumerate()
                    .fold((0, f32::MIN), |best, now| if now.1 > best.1 { now } else { best });
            if class != 0 && class != last {
                if let Some(character) = self.characters.get(class - 1) {
                    text.push_str(character);
                    sure += score;
                    kept += 1;
                }
            }
            last = class;
        }
        let confidence = if kept == 0 { 0.0 } else { sure / kept as f32 };
        let text = text.trim().to_string();
        Ok((!text.is_empty() && confidence >= min_confidence).then_some((text, confidence)))
    }
}

/// Print that is large in the picture comes back word by word. Words on one row with no more than a word space
/// between them are one line again: "РЕПУБЛИКА" "БЪЛГАРИЯ", "Date of" "expiry". A column gap keeps lines apart.
fn join_words(mut words: Vec<Line>, join_gap: f64) -> Vec<Line> {
    let middle = |l: &Line| (l.rect[1] + l.rect[3]) / 2.0;
    let height = |l: &Line| l.rect[3] - l.rect[1];
    words.sort_by(|a, b| a.rect[0].total_cmp(&b.rect[0]));
    let mut lines: Vec<Line> = Vec::new();
    for word in words {
        let follows = |line: &Line| {
            let tall = height(line).min(height(&word));
            let gap = word.rect[0] - line.rect[2];
            (middle(line) - middle(&word)).abs() < 0.35 * tall && gap < join_gap * tall && gap > -0.5 * tall
        };
        match lines.iter_mut().filter(|line| follows(line)).max_by(|a, b| a.rect[2].total_cmp(&b.rect[2])) {
            Some(line) => {
                line.text = format!("{} {}", line.text, word.text);
                line.rect = [
                    line.rect[0],
                    line.rect[1].min(word.rect[1]),
                    word.rect[2].max(line.rect[2]),
                    line.rect[3].max(word.rect[3]),
                ];
                line.confidence = line.confidence.min(word.confidence);
            }
            None => lines.push(word),
        }
    }
    lines
}

/// The picture in an image file, turned the way its camera says it was held.
fn decode(content: &[u8]) -> Result<RgbImage, String> {
    let unreadable = |e: image::ImageError| format!("Cannot read the image: {e}");
    let reader =
        image::ImageReader::new(std::io::Cursor::new(content)).with_guessed_format().map_err(|e| e.to_string())?;
    let mut decoder = reader.into_decoder().map_err(unreadable)?;
    let orientation = decoder.orientation().map_err(unreadable)?;
    let mut picture = image::DynamicImage::from_decoder(decoder).map_err(unreadable)?;
    picture.apply_orientation(orientation);
    Ok(picture.to_rgb8())
}

/// The size a viewer shows the image file at: its own, with width and height swapped when the camera was held
/// sideways.
pub fn shown_size(content: &[u8]) -> Option<(u32, u32)> {
    use image::metadata::Orientation::*;
    let reader = image::ImageReader::new(std::io::Cursor::new(content)).with_guessed_format().ok()?;
    let mut decoder = reader.into_decoder().ok()?;
    let (width, height) = decoder.dimensions();
    let swapped = matches!(decoder.orientation().ok()?, Rotate90 | Rotate270 | Rotate90FlipH | Rotate270FlipH);
    Some(if swapped { (height, width) } else { (width, height) })
}

/// A box of the picture stood upright and laid level, as it lies in the picture as shown (`width` by `height`):
/// the picture leans by `skew` once it stands upright, which takes `turned` quarter turns clockwise.
pub fn shown_rect(rect: [f64; 4], width: f64, height: f64, turned: u8, skew: f64) -> [f64; 4] {
    // back to its lean, about the middle of the upright picture; the box keeps its size
    let (cx, cy) = if turned % 2 == 1 { (height / 2.0, width / 2.0) } else { (width / 2.0, height / 2.0) };
    let (mx, my) = ((rect[0] + rect[2]) / 2.0 - cx, (rect[1] + rect[3]) / 2.0 - cy);
    let (mx, my) = (cx + mx * skew.cos() - my * skew.sin(), cy + mx * skew.sin() + my * skew.cos());
    let (half_w, half_h) = ((rect[2] - rect[0]) / 2.0, (rect[3] - rect[1]) / 2.0);
    let rect = [mx - half_w, my - half_h, mx + half_w, my + half_h];
    let point = |x: f64, y: f64| match turned % 4 {
        1 => (y, height - x),
        2 => (width - x, height - y),
        3 => (width - y, x),
        _ => (x, y),
    };
    let (a, b) = (point(rect[0], rect[1]), point(rect[2], rect[3]));
    [a.0.min(b.0), a.1.min(b.1), a.0.max(b.0), a.1.max(b.1)]
}

/// The middle value of some angles: the lean of a picture, from the leans of its lines. 0 when too few say.
fn middle_of(mut angles: Vec<f64>) -> f64 {
    if angles.len() < 3 {
        return 0.0;
    }
    angles.sort_by(f64::total_cmp);
    angles[angles.len() / 2]
}

/// The connected groups of set pixels, each as its pixels; the mask is cleared as they are taken.
fn regions(mask: &mut [bool], width: usize, height: usize) -> Vec<Vec<(usize, usize)>> {
    let mut found = Vec::new();
    for start in 0..mask.len() {
        if !mask[start] {
            continue;
        }
        mask[start] = false;
        let mut region = vec![(start % width, start / width)];
        let mut next = 0;
        while next < region.len() {
            let (x, y) = region[next];
            next += 1;
            for (dx, dy) in [(-1i64, -1i64), (0, -1), (1, -1), (-1, 0), (1, 0), (-1, 1), (0, 1), (1, 1)] {
                let (nx, ny) = (x as i64 + dx, y as i64 + dy);
                if nx < 0 || ny < 0 || nx >= width as i64 || ny >= height as i64 {
                    continue;
                }
                let at = ny as usize * width + nx as usize;
                if mask[at] {
                    mask[at] = false;
                    region.push((nx as usize, ny as usize));
                }
            }
        }
        found.push(region);
    }
    found
}

/// The tight box around a region, turned to lie along it: a long region gives its own direction (a photographed
/// card is rarely level), a short one leans as `lean` says. Also whether the direction is the region's own.
fn bounding(region: &[(usize, usize)], lean: f64) -> Option<(TextBox, bool)> {
    let n = region.len() as f64;
    if region.len() < 4 {
        return None;
    }
    let (mx, my) = region.iter().fold((0.0, 0.0), |sum, &(x, y)| (sum.0 + x as f64 / n, sum.1 + y as f64 / n));
    let (mut xx, mut xy, mut yy) = (0.0, 0.0, 0.0);
    for &(x, y) in region {
        let (dx, dy) = (x as f64 - mx, y as f64 - my);
        xx += dx * dx / n;
        xy += dx * dy / n;
        yy += dy * dy / n;
    }
    // the direction the region is longest in, when it is clearly longer one way
    let spread = ((xx - yy).powi(2) + 4.0 * xy * xy).sqrt();
    let (long, short) = ((xx + yy + spread) / 2.0, (xx + yy - spread) / 2.0);
    let own = long > 16.0 * short.max(0.05);
    let angle = if own { 0.5 * (2.0 * xy).atan2(xx - yy) } else { lean };
    // text runs left to right: a region that leans more than 45 degrees is taken as level
    let (angle, own) = if angle.abs() > std::f64::consts::FRAC_PI_4 { (0.0, false) } else { (angle, own) };
    let along = (angle.cos(), angle.sin());
    let across = (-along.1, along.0);
    let (mut low, mut high) = ((f64::MAX, f64::MAX), (f64::MIN, f64::MIN));
    for &(x, y) in region {
        let (u, v) = (x as f64 * along.0 + y as f64 * along.1, x as f64 * across.0 + y as f64 * across.1);
        low = (low.0.min(u), low.1.min(v));
        high = (high.0.max(u + 1.0), high.1.max(v + 1.0));
    }
    let (cu, cv) = ((low.0 + high.0) / 2.0, (low.1 + high.1) / 2.0);
    let found = TextBox {
        centre: (cu * along.0 + cv * across.0, cu * along.1 + cv * across.1),
        along,
        half: ((high.0 - low.0) / 2.0, (high.1 - low.1) / 2.0),
    };
    Some((found, own))
}

/// The box cut out of the image and laid level.
fn crop(image: &RgbImage, found: &TextBox) -> RgbImage {
    let (width, height) = (((found.half.0 * 2.0).round() as u32).max(1), ((found.half.1 * 2.0).round() as u32).max(1));
    let [top_left, top_right, _, bottom_left] = found.corners();
    let (max_x, max_y) = (image.width() as f64 - 1.0, image.height() as f64 - 1.0);
    RgbImage::from_fn(width, height, |x, y| {
        let (fx, fy) = ((x as f64 + 0.5) / width as f64, (y as f64 + 0.5) / height as f64);
        let sx =
            (top_left.0 + fx * (top_right.0 - top_left.0) + fy * (bottom_left.0 - top_left.0) - 0.5).clamp(0.0, max_x);
        let sy =
            (top_left.1 + fx * (top_right.1 - top_left.1) + fy * (bottom_left.1 - top_left.1) - 0.5).clamp(0.0, max_y);
        // between the four pixels around the point
        let (x0, y0) = (sx.floor() as u32, sy.floor() as u32);
        let (x1, y1) = ((x0 + 1).min(image.width() - 1), (y0 + 1).min(image.height() - 1));
        let (tx, ty) = (sx - x0 as f64, sy - y0 as f64);
        let mix = |channel: usize| {
            let at = |x: u32, y: u32| image.get_pixel(x, y)[channel] as f64;
            let top = at(x0, y0) * (1.0 - tx) + at(x1, y0) * tx;
            let bottom = at(x0, y1) * (1.0 - tx) + at(x1, y1) * tx;
            (top * (1.0 - ty) + bottom * ty).round() as u8
        };
        image::Rgb([mix(0), mix(1), mix(2)])
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn word(text: &str, x: f64, y: f64, width: f64) -> Line {
        Line { text: text.into(), rect: [x, y, x + width, y + 100.0], confidence: 0.9 }
    }

    #[test]
    fn words_a_space_apart_on_one_row_are_one_line() {
        let words = vec![
            word("БЪЛГАРИЯ", 1685.0, 864.0, 360.0),
            word("РЕПУБЛИКА", 1298.0, 850.0, 350.0),
            word("REPUBLIC", 2301.0, 870.0, 280.0),
            word("ЛИЧНА КАРТА", 1284.0, 960.0, 400.0),
        ];
        let mut lines: Vec<String> = join_words(words, 0.6).into_iter().map(|l| l.text).collect();
        lines.sort();
        assert_eq!(
            lines,
            ["REPUBLIC", "ЛИЧНА КАРТА", "РЕПУБЛИКА БЪЛГАРИЯ"],
            "a column gap and the next row stay apart"
        );
    }
}
