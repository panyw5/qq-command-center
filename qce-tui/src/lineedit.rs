/// 单行文本编辑状态：文本 + 光标（字节偏移，始终落在 UTF-8 字符边界上）。
///
/// 给聊天输入、图片路径、过滤器这类单行输入框提供统一的
/// 光标移动 / 插入 / 删除 / 按词移动语义。
#[derive(Debug, Default, Clone)]
pub struct LineEdit {
    pub text: String,
    /// 光标字节位置，范围 0..=text.len()，必在字符边界。
    cursor: usize,
}

impl LineEdit {
    /// 光标前的文本（渲染算列宽用）。
    pub fn before_cursor(&self) -> &str {
        &self.text[..self.cursor]
    }

    /// 清空文本并把光标归零。
    pub fn clear(&mut self) {
        self.text.clear();
        self.cursor = 0;
    }

    fn prev_boundary(&self) -> usize {
        self.text[..self.cursor]
            .char_indices()
            .last()
            .map_or(0, |(i, _)| i)
    }

    fn next_boundary(&self) -> usize {
        self.text[self.cursor..]
            .chars()
            .next()
            .map_or(self.text.len(), |c| self.cursor + c.len_utf8())
    }

    pub fn move_left(&mut self) {
        self.cursor = self.prev_boundary();
    }

    pub fn move_right(&mut self) {
        self.cursor = self.next_boundary();
    }

    pub fn home(&mut self) {
        self.cursor = 0;
    }

    pub fn end(&mut self) {
        self.cursor = self.text.len();
    }

    /// 光标处插入一个字符。
    pub fn insert(&mut self, c: char) {
        self.text.insert(self.cursor, c);
        self.cursor += c.len_utf8();
    }

    /// 删除光标前一个字符（Backspace）。
    pub fn backspace(&mut self) {
        if self.cursor > 0 {
            let prev = self.prev_boundary();
            self.text.replace_range(prev..self.cursor, "");
            self.cursor = prev;
        }
    }

    /// 删除光标处的字符（Delete 键）。
    pub fn delete(&mut self) {
        if self.cursor < self.text.len() {
            let next = self.next_boundary();
            self.text.replace_range(self.cursor..next, "");
        }
    }

    /// 删除光标前的全部文本（Ctrl+U）。
    pub fn kill_before_cursor(&mut self) {
        self.text.replace_range(..self.cursor, "");
        self.cursor = 0;
    }

    /// 删除光标后的全部文本（Ctrl+K）。
    pub fn kill_after_cursor(&mut self) {
        self.text.truncate(self.cursor);
    }

    /// 删除光标前的一个词（Ctrl+W）。
    pub fn delete_word_back(&mut self) {
        let end = self.cursor;
        self.word_left();
        self.text.replace_range(self.cursor..end, "");
    }

    /// 光标左移一个词（Ctrl/Alt+←）：先跳过非词字符，再跳过词字符。
    pub fn word_left(&mut self) {
        let mut pos = self.cursor;
        let mut chars = self.text[..pos].char_indices().rev().peekable();
        while let Some(&(i, c)) = chars.peek() {
            if is_word_char(c) {
                break;
            }
            pos = i;
            chars.next();
        }
        while let Some(&(i, c)) = chars.peek() {
            if !is_word_char(c) {
                break;
            }
            pos = i;
            chars.next();
        }
        self.cursor = pos;
    }

    /// 光标右移一个词（Ctrl/Alt+→）。
    pub fn word_right(&mut self) {
        let s = &self.text[self.cursor..];
        let mut off = 0usize;
        for c in s.chars() {
            if is_word_char(c) {
                break;
            }
            off += c.len_utf8();
        }
        for c in s[off..].chars() {
            if !is_word_char(c) {
                break;
            }
            off += c.len_utf8();
        }
        self.cursor += off;
    }
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

#[cfg(test)]
mod tests {
    use super::*;

    fn edit(s: &str) -> LineEdit {
        let mut e = LineEdit::default();
        for c in s.chars() {
            e.insert(c);
        }
        e
    }

    #[test]
    fn insert_appends_at_end() {
        let e = edit("hello");
        assert_eq!(e.text, "hello");
        assert_eq!(e.before_cursor(), "hello");
    }

    #[test]
    fn left_right_moves_by_char() {
        let mut e = edit("a中b");
        e.move_left();
        e.move_left();
        assert_eq!(e.before_cursor(), "a");
        e.move_right();
        assert_eq!(e.before_cursor(), "a中");
        e.move_right();
        e.move_right(); // 已到末尾，不动
        assert_eq!(e.before_cursor(), "a中b");
    }

    #[test]
    fn insert_and_backspace_mid_text() {
        let mut e = edit("ac");
        e.move_left();
        e.insert('b');
        assert_eq!(e.text, "abc");
        e.backspace();
        assert_eq!(e.text, "ac");
        assert_eq!(e.before_cursor(), "a");
    }

    #[test]
    fn delete_removes_char_at_cursor() {
        let mut e = edit("abc");
        e.home();
        e.move_right();
        e.delete();
        assert_eq!(e.text, "ac");
    }

    #[test]
    fn home_end_work() {
        let mut e = edit("xyz");
        e.home();
        assert_eq!(e.before_cursor(), "");
        e.end();
        assert_eq!(e.before_cursor(), "xyz");
    }

    #[test]
    fn word_moves() {
        let mut e = edit("foo bar  baz");
        e.word_left();
        assert_eq!(e.before_cursor(), "foo bar  ");
        e.word_left();
        assert_eq!(e.before_cursor(), "foo ");
        e.word_left();
        assert_eq!(e.before_cursor(), "");
        e.word_right();
        assert_eq!(e.before_cursor(), "foo");
        e.word_right();
        assert_eq!(e.before_cursor(), "foo bar");
        e.end();
        e.word_right(); // 末尾不动
        assert_eq!(e.before_cursor(), "foo bar  baz");
    }

    #[test]
    fn delete_word_back_removes_word() {
        let mut e = edit("foo bar");
        e.delete_word_back();
        assert_eq!(e.text, "foo ");
        assert_eq!(e.before_cursor(), "foo ");
    }

    #[test]
    fn multibyte_boundary_safe() {
        let mut e = edit("你好世界");
        e.move_left();
        e.insert('x');
        assert_eq!(e.text, "你好世x界");
        e.backspace();
        assert_eq!(e.text, "你好世界");
    }
}
