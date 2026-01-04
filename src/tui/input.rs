//! Reusable text input component with path-aware navigation
//!
//! Provides consistent text editing across all input fields in kage:
//! - Basic cursor movement (Left/Right/Home/End)
//! - Path-segment navigation (Option+Left/Right)
//! - Character deletion (Backspace)
//! - Path-segment deletion (Option+Backspace)

/// A text input field with cursor position tracking
#[derive(Debug, Clone, Default)]
pub struct TextInput {
    /// The text content
    pub text: String,
    /// Cursor position (in characters, not bytes)
    pub cursor: usize,
}

impl TextInput {
    /// Create a new empty text input
    pub fn new() -> Self {
        Self {
            text: String::new(),
            cursor: 0,
        }
    }

    /// Create a text input with initial value, cursor at end
    pub fn with_text(text: String) -> Self {
        let cursor = text.chars().count();
        Self { text, cursor }
    }

    /// Set text and move cursor to end
    pub fn set_text(&mut self, text: String) {
        self.cursor = text.chars().count();
        self.text = text;
    }

    /// Clear the input
    pub fn clear(&mut self) {
        self.text.clear();
        self.cursor = 0;
    }

    /// Get the text content
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Check if empty
    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    /// Move cursor left by one character
    pub fn move_left(&mut self) {
        self.cursor = self.cursor.saturating_sub(1);
    }

    /// Move cursor right by one character
    pub fn move_right(&mut self) {
        let len = self.text.chars().count();
        if self.cursor < len {
            self.cursor += 1;
        }
    }

    /// Move cursor to start
    pub fn move_home(&mut self) {
        self.cursor = 0;
    }

    /// Move cursor to end
    pub fn move_end(&mut self) {
        self.cursor = self.text.chars().count();
    }

    /// Move cursor to previous word/path boundary (Option+Left)
    pub fn move_word_left(&mut self) {
        if self.cursor == 0 {
            return;
        }

        let chars: Vec<char> = self.text.chars().collect();
        let mut pos = self.cursor.saturating_sub(1);

        // Skip any trailing slashes/spaces at cursor position
        while pos > 0 && is_boundary_char(chars[pos]) {
            pos -= 1;
        }

        // Find the previous boundary or start
        while pos > 0 && !is_boundary_char(chars[pos - 1]) {
            pos -= 1;
        }

        self.cursor = pos;
    }

    /// Move cursor to next word/path boundary (Option+Right)
    pub fn move_word_right(&mut self) {
        let chars: Vec<char> = self.text.chars().collect();
        let len = chars.len();

        if self.cursor >= len {
            return;
        }

        let mut pos = self.cursor;

        // Skip current character if it's a boundary
        if pos < len && is_boundary_char(chars[pos]) {
            pos += 1;
        }

        // Find the next boundary or end
        while pos < len && !is_boundary_char(chars[pos]) {
            pos += 1;
        }

        self.cursor = pos;
    }

    /// Insert a character at cursor position
    pub fn insert_char(&mut self, c: char) {
        let char_count = self.text.chars().count();
        if self.cursor <= char_count {
            let byte_pos = self.text.char_indices()
                .nth(self.cursor)
                .map(|(i, _)| i)
                .unwrap_or(self.text.len());
            self.text.insert(byte_pos, c);
            self.cursor += 1;
        }
    }

    /// Delete character before cursor (Backspace)
    pub fn delete_char(&mut self) {
        if self.cursor > 0 {
            self.cursor -= 1;
            let char_count = self.text.chars().count();
            if !self.text.is_empty() && self.cursor < char_count {
                let byte_pos = self.text.char_indices()
                    .nth(self.cursor)
                    .map(|(i, _)| i)
                    .unwrap_or(0);
                self.text.remove(byte_pos);
            }
        }
    }

    /// Delete previous word/path segment (Option+Backspace)
    pub fn delete_word_left(&mut self) {
        if self.cursor == 0 {
            return;
        }

        let old_cursor = self.cursor;
        let chars: Vec<char> = self.text.chars().collect();
        let mut new_pos = self.cursor.saturating_sub(1);

        // Skip any trailing boundary chars at cursor position
        while new_pos > 0 && is_boundary_char(chars[new_pos]) {
            new_pos -= 1;
        }

        // Find the previous boundary or start
        while new_pos > 0 && !is_boundary_char(chars[new_pos - 1]) {
            new_pos -= 1;
        }

        // Delete the range [new_pos, old_cursor)
        let start_byte = self.text.char_indices()
            .nth(new_pos)
            .map(|(i, _)| i)
            .unwrap_or(0);
        let end_byte = self.text.char_indices()
            .nth(old_cursor)
            .map(|(i, _)| i)
            .unwrap_or(self.text.len());

        self.text.replace_range(start_byte..end_byte, "");
        self.cursor = new_pos;
    }

    /// Render the text with cursor for display
    /// Returns (before_cursor, after_cursor) for styling
    pub fn render_parts(&self) -> (String, String) {
        let char_count = self.text.chars().count();
        if self.cursor < char_count {
            let before: String = self.text.chars().take(self.cursor).collect();
            let after: String = self.text.chars().skip(self.cursor).collect();
            (before, after)
        } else {
            (self.text.clone(), String::new())
        }
    }

    /// Render the text with a cursor character inserted
    pub fn render_with_cursor(&self, prefix: &str, cursor_char: char) -> String {
        let (before, after) = self.render_parts();
        if after.is_empty() {
            format!("{}{}{}", prefix, before, cursor_char)
        } else {
            format!("{}{}{}{}", prefix, before, cursor_char, after)
        }
    }
}

/// Check if a character is a word/path boundary
fn is_boundary_char(c: char) -> bool {
    c == '/' || c == '\\' || c == ' ' || c == '-' || c == '_' || c == '.'
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic_operations() {
        let mut input = TextInput::new();
        assert!(input.is_empty());

        input.insert_char('a');
        input.insert_char('b');
        input.insert_char('c');
        assert_eq!(input.text(), "abc");
        assert_eq!(input.cursor, 3);

        input.delete_char();
        assert_eq!(input.text(), "ab");
        assert_eq!(input.cursor, 2);
    }

    #[test]
    fn test_cursor_movement() {
        let mut input = TextInput::with_text("/usr/local/bin".to_string());
        assert_eq!(input.cursor, 14);

        input.move_home();
        assert_eq!(input.cursor, 0);

        input.move_end();
        assert_eq!(input.cursor, 14);

        input.move_left();
        assert_eq!(input.cursor, 13);

        input.move_right();
        assert_eq!(input.cursor, 14);
    }

    #[test]
    fn test_word_navigation() {
        let mut input = TextInput::with_text("/usr/local/bin".to_string());

        // At end, move word left should go to start of "bin"
        input.move_word_left();
        assert_eq!(input.cursor, 11); // after the last /

        input.move_word_left();
        assert_eq!(input.cursor, 5); // after /usr/

        input.move_word_right();
        assert_eq!(input.cursor, 10); // at /local/
    }

    #[test]
    fn test_delete_word() {
        let mut input = TextInput::with_text("/usr/local/bin".to_string());

        input.delete_word_left();
        assert_eq!(input.text(), "/usr/local/");
        assert_eq!(input.cursor, 11);

        input.delete_word_left();
        assert_eq!(input.text(), "/usr/");
        assert_eq!(input.cursor, 5);
    }

    #[test]
    fn test_render_parts() {
        let mut input = TextInput::with_text("hello".to_string());
        input.cursor = 2;

        let (before, after) = input.render_parts();
        assert_eq!(before, "he");
        assert_eq!(after, "llo");
    }
}
