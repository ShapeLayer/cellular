//! The full-screen editor used for list-valued config fields.
//!
//! One array element per line, nano-like: enter inserts a line, esc cancels and
//! ctrl+s keeps the change.

#[derive(Debug, Clone)]
pub struct ArrayEditor {
    pub field: &'static str,
    pub lines: Vec<String>,
    pub row: usize,
    /// Cursor position inside the row, in characters.
    pub column: usize,
    /// First visible row.
    pub scroll: usize,
}

impl ArrayEditor {
    pub fn new(field: &'static str, items: &[String]) -> Self {
        let lines = if items.is_empty() {
            vec![String::new()]
        } else {
            items.to_vec()
        };
        ArrayEditor {
            field,
            lines,
            row: 0,
            column: 0,
            scroll: 0,
        }
    }

    fn row_len(&self) -> usize {
        self.lines[self.row].chars().count()
    }

    fn byte_index(&self, column: usize) -> usize {
        self.lines[self.row]
            .char_indices()
            .nth(column)
            .map(|(index, _)| index)
            .unwrap_or(self.lines[self.row].len())
    }

    pub fn insert_char(&mut self, character: char) {
        let at = self.byte_index(self.column);
        self.lines[self.row].insert(at, character);
        self.column += 1;
    }

    pub fn insert_str(&mut self, text: &str) {
        for character in text.chars() {
            if character == '\n' {
                self.newline();
            } else {
                self.insert_char(character);
            }
        }
    }

    pub fn backspace(&mut self) {
        if self.column > 0 {
            let at = self.byte_index(self.column - 1);
            self.lines[self.row].remove(at);
            self.column -= 1;
        } else if self.row > 0 {
            // Join with the previous line.
            let current = self.lines.remove(self.row);
            self.row -= 1;
            self.column = self.row_len();
            self.lines[self.row].push_str(&current);
        }
    }

    pub fn delete(&mut self) {
        if self.column < self.row_len() {
            let at = self.byte_index(self.column);
            self.lines[self.row].remove(at);
        } else if self.row + 1 < self.lines.len() {
            let next = self.lines.remove(self.row + 1);
            self.lines[self.row].push_str(&next);
        }
    }

    pub fn newline(&mut self) {
        let at = self.byte_index(self.column);
        let tail = self.lines[self.row].split_off(at);
        self.lines.insert(self.row + 1, tail);
        self.row += 1;
        self.column = 0;
    }

    pub fn move_up(&mut self) {
        if self.row > 0 {
            self.row -= 1;
            self.column = self.column.min(self.row_len());
        }
    }

    pub fn move_down(&mut self) {
        if self.row + 1 < self.lines.len() {
            self.row += 1;
            self.column = self.column.min(self.row_len());
        }
    }

    pub fn move_left(&mut self) {
        if self.column > 0 {
            self.column -= 1;
        } else if self.row > 0 {
            self.row -= 1;
            self.column = self.row_len();
        }
    }

    pub fn move_right(&mut self) {
        if self.column < self.row_len() {
            self.column += 1;
        } else if self.row + 1 < self.lines.len() {
            self.row += 1;
            self.column = 0;
        }
    }

    pub fn move_home(&mut self) {
        self.column = 0;
    }

    pub fn move_end(&mut self) {
        self.column = self.row_len();
    }

    /// Keep the cursor row inside a window of `height` rows.
    pub fn scroll_into_view(&mut self, height: usize) {
        if height == 0 {
            return;
        }
        if self.row < self.scroll {
            self.scroll = self.row;
        } else if self.row >= self.scroll + height {
            self.scroll = self.row + 1 - height;
        }
    }

    /// The edited value: blank lines are dropped, as they carry no element.
    pub fn value(&self) -> Vec<String> {
        self.lines
            .iter()
            .map(|line| line.trim().to_string())
            .filter(|line| !line.is_empty())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn editor() -> ArrayEditor {
        ArrayEditor::new("metric", &["lines".to_string(), "chars".to_string()])
    }

    #[test]
    fn splitting_and_joining_lines() {
        let mut editor = editor();
        editor.move_end();
        editor.newline();
        assert_eq!(editor.lines, vec!["lines", "", "chars"]);
        editor.backspace();
        assert_eq!(editor.lines, vec!["lines", "chars"]);
        assert_eq!((editor.row, editor.column), (0, 5));
    }

    #[test]
    fn blank_lines_are_not_elements() {
        let mut editor = ArrayEditor::new("metric", &[]);
        editor.insert_str("lines\n\nchars");
        assert_eq!(editor.value(), vec!["lines", "chars"]);
    }

    #[test]
    fn scrolling_follows_the_cursor() {
        let mut editor = ArrayEditor::new(
            "metric",
            &(0..20).map(|index| index.to_string()).collect::<Vec<_>>(),
        );
        editor.row = 15;
        editor.scroll_into_view(5);
        assert_eq!(editor.scroll, 11);
        editor.row = 2;
        editor.scroll_into_view(5);
        assert_eq!(editor.scroll, 2);
    }
}
