use crate::detector::FaceBox;
use image::DynamicImage;
use std::io::{self, Write};
use crossterm::{
    event::{self, Event, KeyCode, KeyEvent},
    terminal::{self, ClearType},
    cursor,
    execute,
};

const ASCII_RAMP: &str = " .·:;+=xX#@";
const DEFAULT_WIDTH: usize = 80;
const DEFAULT_HEIGHT: usize = 30;

pub struct AsciiRenderer {
    width: usize,
    height: usize,
}

impl AsciiRenderer {
    pub fn new(width: Option<usize>, height: Option<usize>) -> Self {
        // Get actual terminal size if not specified
        let (term_width, term_height) = terminal::size()
            .map(|(w, h)| (w as usize, h as usize))
            .unwrap_or((DEFAULT_WIDTH, DEFAULT_HEIGHT));
        
        // Cut resolution in half for better performance
        Self {
            width: width.unwrap_or((term_width / 2).min(DEFAULT_WIDTH / 2)),
            height: height.unwrap_or((term_height.saturating_sub(5) / 2).min(DEFAULT_HEIGHT / 2)),
        }
    }
    
    pub fn height(&self) -> usize {
        self.height
    }

    pub fn render_frame_with_progress(
        &self,
        image: &DynamicImage,
        faces: &[FaceBox],
        captured: usize,
        total: usize,
    ) -> String {
        let mut grid = self.image_to_ascii(image);
        
        if let Some(face) = faces.first() {
            // Scale face coordinates to terminal size
            let img_width = image.width() as f32;
            let img_height = image.height() as f32;
            
            let face_x1 = ((face.x1 / img_width) * self.width as f32) as usize;
            let face_x2 = ((face.x2 / img_width) * self.width as f32) as usize;
            let face_y1 = ((face.y1 / img_height) * self.height as f32) as usize;
            
            // Center everything above the face box
            let face_center_x = (face_x1 + face_x2) / 2;
            
            // Draw message first, 2 lines above face
            let msg = if captured < total { "Move head slightly" } else { "Complete!" };
            let msg_x = face_center_x.saturating_sub(msg.len() / 2) + 10;  // Add 4 spaces offset to the right
            let msg_y = face_y1.saturating_sub(2).max(0);
            self.overlay_text(&mut grid, msg, msg_x, msg_y);
            
            // Draw progress bar directly above face box (1 line above)
            let bar = self.create_progress_bar(captured, total);
            let bar_len = bar.len();
            let bar_x = face_center_x.saturating_sub(bar_len / 2) + 12;  // Add 4 spaces offset to the right
            let bar_y = face_y1.saturating_sub(1).max(0);
            self.overlay_text(&mut grid, &bar, bar_x, bar_y);
            
            // Draw face detection box
            self.draw_face_box(&mut grid, face, img_width, img_height);
        }
        // No else branch - just show the ASCII art without any message when no face is detected
        // This prevents flashing when face detection temporarily fails between frames
        
        self.grid_to_string(&grid)
    }

    fn image_to_ascii(&self, image: &DynamicImage) -> Vec<Vec<char>> {
        let mut grid = vec![vec![' '; self.width]; self.height];
        
        // Convert to grayscale
        let gray = image.to_luma8();
        let (img_width, img_height) = gray.dimensions();
        
        // Account for terminal character aspect ratio (chars are ~2x taller than wide)
        // So we need to sample the image differently to maintain aspect ratio
        for term_y in 0..self.height {
            for term_x in 0..self.width {
                // Sample from the original image
                let img_x = (term_x as f32 / self.width as f32 * img_width as f32) as u32;
                let img_y = (term_y as f32 / self.height as f32 * img_height as f32) as u32;
                
                if img_x < img_width && img_y < img_height {
                    let pixel = gray.get_pixel(img_x, img_y);
                    let brightness = pixel[0];
                    
                    // Map brightness to ASCII character  
                    let char_idx = (brightness as usize * (ASCII_RAMP.len() - 1)) / 255;
                    grid[term_y][term_x] = ASCII_RAMP.chars().nth(char_idx).unwrap_or(' ');
                }
            }
        }
        
        grid
    }

    fn create_progress_bar(&self, captured: usize, _total: usize) -> String {
        // Simple 5 box progress - one per capture
        let filled = "■".repeat(captured.min(5));
        let empty = "□".repeat(5_usize.saturating_sub(captured));
        
        format!("[{}{}]", filled, empty)
    }

    fn overlay_text(&self, grid: &mut Vec<Vec<char>>, text: &str, center_x: usize, y: usize) {
        if y >= self.height {
            return;
        }
        
        let text_len = text.len();
        let start_x = center_x.saturating_sub(text_len / 2);
        
        for (i, ch) in text.chars().enumerate() {
            let x = start_x + i;
            if x < self.width {
                grid[y][x] = ch;
            }
        }
    }

    fn overlay_center_text(&self, grid: &mut Vec<Vec<char>>, text: &str) {
        let center_y = self.height / 2;
        let center_x = self.width / 2;
        self.overlay_text(grid, text, center_x, center_y);
    }

    fn draw_face_box(&self, grid: &mut Vec<Vec<char>>, face: &FaceBox, img_width: f32, img_height: f32) {
        // Scale face coordinates to terminal
        let x1 = ((face.x1 / img_width) * self.width as f32) as usize;
        let x2 = ((face.x2 / img_width) * self.width as f32) as usize;
        let y1 = ((face.y1 / img_height) * self.height as f32) as usize;
        let y2 = ((face.y2 / img_height) * self.height as f32) as usize;
        
        // Draw corners
        if y1 < self.height && x1 < self.width {
            grid[y1][x1] = '┌';
        }
        if y1 < self.height && x2 < self.width {
            grid[y1][x2.saturating_sub(1)] = '┐';
        }
        if y2 < self.height && x1 < self.width {
            grid[y2.saturating_sub(1)][x1] = '└';
        }
        if y2 < self.height && x2 < self.width {
            grid[y2.saturating_sub(1)][x2.saturating_sub(1)] = '┘';
        }
        
        // Draw horizontal lines
        for x in (x1 + 1)..(x2.saturating_sub(1)).min(self.width) {
            if y1 < self.height {
                grid[y1][x] = '─';
            }
            if y2.saturating_sub(1) < self.height {
                grid[y2.saturating_sub(1)][x] = '─';
            }
        }
        
        // Draw vertical lines
        for y in (y1 + 1)..(y2.saturating_sub(1)).min(self.height) {
            if x1 < self.width {
                grid[y][x1] = '│';
            }
            if x2.saturating_sub(1) < self.width {
                grid[y][x2.saturating_sub(1)] = '│';
            }
        }
    }

    fn grid_to_string(&self, grid: &Vec<Vec<char>>) -> String {
        grid.iter()
            .map(|row| {
                // Ensure each row is exactly the right width
                let line: String = row.iter().take(self.width).collect();
                // Pad with spaces if needed (shouldn't happen but just in case)
                if line.len() < self.width {
                    format!("{:width$}", line, width = self.width)
                } else {
                    line
                }
            })
            .collect::<Vec<_>>()
            .join("\r\n")  // Use explicit carriage return + newline
    }
}

pub fn clear_screen() -> io::Result<()> {
    crossterm::execute!(
        io::stdout(),
        terminal::Clear(ClearType::All),
        cursor::MoveTo(0, 0)
    )?;
    io::stdout().flush()
}

pub fn check_for_escape() -> io::Result<bool> {
    if event::poll(std::time::Duration::from_millis(0))? {
        if let Event::Key(KeyEvent { code, .. }) = event::read()? {
            return Ok(code == KeyCode::Esc);
        }
    }
    Ok(false)
}

pub fn show_capture_flash() {
    println!("\n    📸 CAPTURED!");
    std::thread::sleep(std::time::Duration::from_millis(200));
}