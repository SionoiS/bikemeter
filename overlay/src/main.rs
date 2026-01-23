//! BikeMeter Overlay Application
//!
//! Desktop application for creating video overlays with telemetry data.

use anyhow::{Context, Result};
use clap::Parser;
use csv::ReaderBuilder;
use image::{ImageBuffer, Rgba, RgbaImage};
use indicatif::{ProgressBar, ProgressStyle};
use serde::Deserialize;
use std::collections::VecDeque;
use std::fs::File;
use std::path::PathBuf;
use tracing::{error, info};

// ============================================================================
// Command Line Arguments
// ============================================================================

#[derive(Parser, Debug, Clone)]
#[command(name = "bikemeter-overlay")]
#[command(author = "BikeMeter")]
#[command(version = env!("CARGO_PKG_VERSION"))]
#[command(about = "Create video overlays with bike telemetry data", long_about = None)]
struct Args {
    /// Input video file
    #[arg(short, long)]
    input: PathBuf,

    /// CSV data file with sensor readings
    #[arg(short, long)]
    data: PathBuf,

    /// Output video file
    #[arg(short, long)]
    output: PathBuf,

    /// Start time offset (milliseconds) to sync with video
    #[arg(short = 't', long, default_value_t = 0)]
    time_offset: i64,

    /// Show preview frames (useful for debugging)
    #[arg(long, default_value_t = false)]
    preview: bool,

    /// Sampling rate of sensor data (Hz)
    #[arg(long, default_value_t = 100)]
    sample_rate: u32,
}

// ============================================================================
// Sensor Data Structure
// ============================================================================

#[derive(Debug, Deserialize, Clone)]
struct SensorReading {
    #[serde(rename = "timestamp_ms")]
    timestamp: u64,
    #[serde(rename = "accel_x")]
    accel_x: f32,
    #[serde(rename = "accel_y")]
    accel_y: f32,
    #[serde(rename = "accel_z")]
    accel_z: f32,
    #[serde(rename = "pitch")]
    pitch: f32,
    #[serde(rename = "roll")]
    roll: f32,
    #[serde(rename = "yaw")]
    yaw: f32,
    #[serde(rename = "quat_w")]
    quat_w: f32,
    #[serde(rename = "quat_x")]
    quat_x: f32,
    #[serde(rename = "quat_y")]
    quat_y: f32,
    #[serde(rename = "quat_z")]
    quat_z: f32,
}

impl SensorReading {
    /// Calculate g-force magnitude from acceleration
    fn g_force(&self) -> f32 {
        (self.accel_x.powi(2) + self.accel_y.powi(2) + self.accel_z.powi(2)).sqrt()
    }
}

// ============================================================================
// Video Processor
// ============================================================================

struct VideoProcessor {
    args: Args,
    sensor_data: Vec<SensorReading>,
}

impl VideoProcessor {
    fn new(args: Args, sensor_data: Vec<SensorReading>) -> Self {
        Self { args, sensor_data }
    }

    /// Load and parse sensor data from CSV file
    fn load_data(path: &PathBuf) -> Result<Vec<SensorReading>> {
        info!("Loading sensor data from: {:?}", path);
        let file = File::open(path).context("Failed to open data file")?;
        let mut rdr = ReaderBuilder::new().from_reader(file);
        let data: Result<Vec<_>, _> = rdr.deserialize().collect();
        let data = data.context("Failed to parse CSV data")?;
        info!("Loaded {} sensor readings", data.len());
        Ok(data)
    }

    /// Find sensor reading closest to a given timestamp
    fn find_reading_at_time(&self, video_ms: f64) -> Option<&SensorReading> {
        let sensor_time_ms = video_ms + self.args.time_offset as f64;

        let idx = self
            .sensor_data
            .binary_search_by(|r| {
                r.timestamp
                    .partial_cmp(&(sensor_time_ms as u64))
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .unwrap_or_else(|i| i);

        if idx == 0 {
            return self.sensor_data.first();
        }
        if idx >= self.sensor_data.len() {
            return self.sensor_data.last();
        }

        let before = &self.sensor_data[idx - 1];
        let after = &self.sensor_data[idx];

        let before_diff = (before.timestamp as f64 - sensor_time_ms).abs();
        let after_diff = (after.timestamp as f64 - sensor_time_ms).abs();

        if before_diff < after_diff {
            Some(before)
        } else {
            Some(after)
        }
    }

    /// Interpolate sensor data for smooth graphs
    fn interpolate_reading(&self, video_ms: f64, window_size: usize) -> InterpolatedData {
        let sensor_time_ms = video_ms + self.args.time_offset as f64;

        let start_idx = self
            .sensor_data
            .partition_point(|r| r.timestamp < (sensor_time_ms - window_size as f64) as u64);

        let window: Vec<_> = self
            .sensor_data
            .iter()
            .skip(start_idx.saturating_sub(window_size))
            .take(window_size * 2)
            .collect();

        let current = self.find_reading_at_time(video_ms);

        let mut max_g: f32 = 0.0;
        let mut g_history = VecDeque::with_capacity(window_size);

        for reading in window.iter() {
            let g = reading.g_force();
            max_g = max_g.max(g);
            g_history.push_back(g);
        }

        InterpolatedData {
            current: current.cloned(),
            max_g,
            history: g_history,
        }
    }

    /// Generate overlay frame with telemetry
    fn generate_overlay(&self, width: u32, height: u32, video_ms: f64) -> Result<RgbaImage> {
        let data = self.interpolate_reading(video_ms, 50);

        // Create image buffer
        let mut imgbuf: RgbaImage = ImageBuffer::new(width, height);

        // Fill with semi-transparent black background
        for pixel in imgbuf.pixels_mut() {
            *pixel = Rgba([0, 0, 0, 180]);
        }

        if let Some(reading) = &data.current {
            let margin = 10u32;
            let gauge_size = 120u32;
            let gauge_y = height - gauge_size - margin;

            // Draw G-Force Gauge (bottom right)
            self.draw_g_force_gauge(
                &mut imgbuf,
                width - gauge_size - margin,
                gauge_y,
                gauge_size,
                reading.g_force(),
                data.max_g,
            );

            // Draw Pitch/Roll Indicator (bottom left)
            self.draw_attitude_indicator(
                &mut imgbuf,
                margin,
                gauge_y,
                gauge_size,
                reading.pitch,
                reading.roll,
            );

            // Draw G-Force Graph (top)
            let graph_height = 100u32;
            self.draw_g_force_graph(
                &mut imgbuf,
                margin,
                margin,
                width - 2 * margin,
                graph_height,
                &data.history,
            );
        }

        Ok(imgbuf)
    }

    /// Draw g-force gauge using image operations
    fn draw_g_force_gauge(
        &self,
        imgbuf: &mut RgbaImage,
        x: u32,
        y: u32,
        size: u32,
        current_g: f32,
        _max_g: f32,
    ) {
        let center_x = x + size / 2;
        let center_y = y + size / 2;
        let radius = (size / 2 - 5) as i32;

        // Draw gauge background circle
        self.draw_circle(imgbuf, center_x, center_y, radius, [60, 60, 60, 200]);

        // Draw gauge arc (180 degrees)
        let g_range = 3.0f32;
        for i in 0..=30 {
            let g = (i as f32 / 10.0).min(g_range);
            let angle_deg = 180.0 + (g / g_range) * 180.0;
            let angle_rad = angle_deg * std::f32::consts::PI / 180.0;

            let x1 = center_x as f32 + radius as f32 * angle_rad.cos() * 0.85;
            let y1 = center_y as f32 + radius as f32 * angle_rad.sin() * 0.85;
            let x2 = center_x as f32 + radius as f32 * angle_rad.cos() * 0.95;
            let y2 = center_y as f32 + radius as f32 * angle_rad.sin() * 0.95;

            let color = if g > 2.0 {
                [255, 50, 50, 255]
            } else if g > 1.0 {
                [255, 200, 50, 255]
            } else {
                [50, 255, 50, 255]
            };
            self.draw_line(imgbuf, x1 as i32, y1 as i32, x2 as i32, y2 as i32, color, 2);
        }

        // Draw needle
        let needle_angle_deg = 180.0 + (current_g.min(g_range) / g_range) * 180.0;
        let needle_angle_rad = needle_angle_deg * std::f32::consts::PI / 180.0;
        let needle_x = center_x as f32 + (radius as f32 - 10.0) * needle_angle_rad.cos();
        let needle_y = center_y as f32 + (radius as f32 - 10.0) * needle_angle_rad.sin();

        self.draw_line(
            imgbuf,
            center_x as i32,
            center_y as i32,
            needle_x as i32,
            needle_y as i32,
            [255, 50, 50, 255],
            3,
        );

        // Draw center circle
        self.draw_circle(imgbuf, center_x, center_y, 5, [255, 50, 50, 255]);
    }

    /// Draw pitch/roll attitude indicator
    fn draw_attitude_indicator(
        &self,
        imgbuf: &mut RgbaImage,
        x: u32,
        y: u32,
        size: u32,
        pitch: f32,
        roll: f32,
    ) {
        let center_x = x + size / 2;
        let center_y = y + size / 2;
        let radius = (size / 2 - 5) as i32;

        // Draw outer circle
        self.draw_circle(imgbuf, center_x, center_y, radius, [60, 60, 60, 200]);

        // Draw pitch ladder
        for pitch_line in [-20.0, -10.0, 0.0, 10.0, 20.0] {
            let y_offset = ((pitch_line - pitch) * 2.0) as i32;
            if y_offset.abs() < radius {
                let line_width = if pitch_line == 0.0 { radius } else { (radius as f32 * 0.6) as i32 };
                self.draw_line(
                    imgbuf,
                    center_x as i32 - line_width / 2,
                    center_y as i32 + y_offset,
                    center_x as i32 + line_width / 2,
                    center_y as i32 + y_offset,
                    [200, 200, 200, 255],
                    2,
                );
            }
        }

        // Draw roll indicator
        let roll_rad = roll * std::f32::consts::PI / 180.0;
        let roll_x = center_x as f32 + (radius as f32 - 15.0) * roll_rad.cos();
        let roll_y = center_y as f32 + (radius as f32 - 15.0) * roll_rad.sin();

        self.draw_circle(imgbuf, roll_x as u32, roll_y as u32, 8, [255, 255, 50, 255]);

        // Draw triangle pointing up (reference)
        let pts = [
            (center_x as i32, (center_y as i32 - radius + 5)),
            (center_x as i32 - 10, center_y as i32 - radius + 15),
            (center_x as i32 + 10, center_y as i32 - radius + 15),
        ];
        self.draw_triangle(imgbuf, &pts, [255, 255, 50, 255]);
    }

    /// Draw g-force history graph
    fn draw_g_force_graph(
        &self,
        imgbuf: &mut RgbaImage,
        x: u32,
        y: u32,
        width: u32,
        height: u32,
        history: &VecDeque<f32>,
    ) {
        // Draw background
        self.draw_rect(imgbuf, x, y, width, height, [20, 20, 20, 150]);

        if history.len() < 2 {
            return;
        }

        // Draw line graph
        let mut prev: Option<(i32, i32)> = None;
        for (i, &g) in history.iter().enumerate() {
            let px = x as i32 + (i as f32 / history.len() as f32 * width as f32) as i32;
            let py = y as i32 + height as i32 - ((g / 3.0).min(1.0) * height as f32) as i32;

            if let Some((prev_x, prev_y)) = prev {
                self.draw_line(imgbuf, prev_x, prev_y, px, py, [50, 200, 255, 255], 2);
            }
            prev = Some((px, py));
        }

        // Draw grid lines
        for g in [1.0, 2.0, 3.0] {
            let gy = y as i32 + height as i32 - (g / 3.0 * height as f32) as i32;
            self.draw_line(
                imgbuf,
                x as i32,
                gy,
                (x + width) as i32,
                gy,
                [100, 100, 100, 100],
                1,
            );
        }
    }

    /// Helper: Draw a circle
    fn draw_circle(&self, imgbuf: &mut RgbaImage, cx: u32, cy: u32, radius: i32, color: [u8; 4]) {
        let r = radius as f32;
        let r2 = r * r;
        for dy in -radius..=radius {
            for dx in -radius..=radius {
                if dx * dx + dy * dy <= r2 as i32 {
                    let px = (cx as i32 + dx) as u32;
                    let py = (cy as i32 + dy) as u32;
                    if px < imgbuf.width() && py < imgbuf.height() {
                        imgbuf.put_pixel(px, py, Rgba(color));
                    }
                }
            }
        }
    }

    /// Helper: Draw a line
    fn draw_line(&self, imgbuf: &mut RgbaImage, x1: i32, y1: i32, x2: i32, y2: i32, color: [u8; 4], width: i32) {
        let dx = (x2 - x1).abs();
        let dy = (y1 - y2).abs();
        let sx = if x1 < x2 { 1 } else { -1 };
        let sy = if y1 < y2 { 1 } else { -1 };
        let mut err = dx - dy;

        let mut x = x1;
        let mut y = y1;

        let width = width as f32;

        loop {
            // Draw a circle at each point for line thickness
            for ox in -(width as i32)..=(width as i32) {
                for oy in -(width as i32)..=(width as i32) {
                    if ox * ox + oy * oy <= width as i32 * width as i32 {
                        let px = (x + ox) as u32;
                        let py = (y + oy) as u32;
                        if px < imgbuf.width() && py < imgbuf.height() {
                            let pixel = imgbuf.get_pixel(px, py);
                            // Alpha blend
                            let src_a = color[3] as f32 / 255.0;
                            let dst_a = pixel[3] as f32 / 255.0;
                            let out_a = src_a + dst_a * (1.0 - src_a);
                            if out_a > 0.0 {
                                let mut blended = pixel.0;
                                for c in 0..3 {
                                    blended[c] = ((color[c] as f32 * src_a
                                        + pixel[c] as f32 * dst_a * (1.0 - src_a))
                                        / out_a) as u8;
                                }
                                blended[3] = (out_a * 255.0) as u8;
                                imgbuf.put_pixel(px, py, Rgba(blended));
                            } else {
                                imgbuf.put_pixel(px, py, Rgba(color));
                            }
                        }
                    }
                }
            }

            if x == x2 && y == y2 {
                break;
            }

            let e2 = 2 * err;
            if e2 > -dy {
                err -= dy;
                x += sx;
            }
            if e2 < dx {
                err += dx;
                y += sy;
            }
        }
    }

    /// Helper: Draw a rectangle
    fn draw_rect(&self, imgbuf: &mut RgbaImage, x: u32, y: u32, width: u32, height: u32, color: [u8; 4]) {
        for py in y..(y + height).min(imgbuf.height()) {
            for px in x..(x + width).min(imgbuf.width()) {
                imgbuf.put_pixel(px, py, Rgba(color));
            }
        }
    }

    /// Helper: Draw a filled triangle
    fn draw_triangle(&self, imgbuf: &mut RgbaImage, pts: &[(i32, i32); 3], color: [u8; 4]) {
        // Simple triangle fill using barycentric coordinates
        let (x0, y0) = pts[0];
        let (x1, y1) = pts[1];
        let (x2, y2) = pts[2];

        let min_x = x0.min(x1).min(x2);
        let max_x = x0.max(x1).max(x2);
        let min_y = y0.min(y1).min(y2);
        let max_y = y0.max(y1).max(y2);

        let denom = (y1 - y2) * (x0 - x2) + (x2 - x1) * (y0 - y2);
        if denom == 0 {
            return;
        }

        for py in min_y..=max_y {
            for px in min_x..=max_x {
                let w0 = ((y1 - y2) * (px - x2) + (x2 - x1) * (py - y2)) as f32 / denom as f32;
                let w1 = ((y2 - y0) * (px - x2) + (x0 - x2) * (py - y2)) as f32 / denom as f32;
                let w2 = 1.0 - w0 - w1;

                if w0 >= 0.0 && w1 >= 0.0 && w2 >= 0.0 {
                    let px = px as u32;
                    let py = py as u32;
                    if px < imgbuf.width() && py < imgbuf.height() {
                        imgbuf.put_pixel(px, py, Rgba(color));
                    }
                }
            }
        }
    }

    /// Process the video
    fn process_video(&self) -> Result<()> {
        info!("Processing video...");
        info!("  Input: {:?}", self.args.input);
        info!("  Output: {:?}", self.args.output);
        info!("  Data: {} readings", self.sensor_data.len());
        info!("  Time offset: {} ms", self.args.time_offset);

        // Export preview frames
        self.export_preview_frames()?;

        info!("Note: Full video compositing requires FFmpeg.");
        info!("Preview frames saved to: preview_frames/");

        Ok(())
    }

    /// Export preview frames
    fn export_preview_frames(&self) -> Result<()> {
        let preview_dir = "preview_frames";
        std::fs::create_dir_all(preview_dir)?;

        let width = 1920u32;
        let height = 1080u32;

        let pb = ProgressBar::new(30);
        pb.set_style(
            ProgressStyle::default_bar()
                .template("[{elapsed_precise}] {bar:40.cyan/blue} {pos}/{len} ({eta})")
                .unwrap()
                .progress_chars("##-"),
        );

        // Export 30 frames at 1-second intervals
        for i in 0..30 {
            let video_ms = (i as f32 * 1000.0) as f64;
            let overlay = self.generate_overlay(width, height, video_ms)?;

            let filename = format!("{}/frame_{:04}.png", preview_dir, i);
            overlay.save(&filename)?;

            pb.inc(1);

            if self.args.preview {
                info!("Saved preview frame: {}", filename);
            }
        }

        pb.finish_with_message("Processing complete!");

        Ok(())
    }
}

// ============================================================================
// Interpolated Data
// ============================================================================

struct InterpolatedData {
    current: Option<SensorReading>,
    max_g: f32,
    history: VecDeque<f32>,
}

// ============================================================================
// Main Entry Point
// ============================================================================

fn main() -> Result<()> {
    // Initialize tracing
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive(tracing::Level::INFO.into()),
        )
        .init();

    let args = Args::parse();
    let start_time = std::time::Instant::now();

    info!("BikeMeter Overlay v{}", env!("CARGO_PKG_VERSION"));
    info!("===========================================");

    // Load sensor data
    let sensor_data = VideoProcessor::load_data(&args.data)?;

    // Create processor
    let processor = VideoProcessor::new(args.clone(), sensor_data);

    // Process video
    if let Err(e) = processor.process_video() {
        error!("Processing failed: {:?}", e);
        return Err(e);
    }

    let elapsed = start_time.elapsed();
    info!("Completed in {:.2} seconds", elapsed.as_secs_f64());

    Ok(())
}
