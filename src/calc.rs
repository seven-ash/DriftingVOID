// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Ash (seven-ash). Additional terms (AGPL-3.0 section 7): see NOTICE.

//! Tiny calculator for number fields: + - * / ^ (right-assoc), parentheses,
//! unary minus. `40*1.5`, `2^3+1`, `(12-4)/2`.

pub fn parse(s: &str) -> Option<f64> {
    let s: Vec<char> = s.chars().filter(|c| !c.is_whitespace()).map(|c| if c == ',' { '.' } else { c }).collect();
    if s.is_empty() {
        // a cleared field means zero
        return Some(0.0);
    }
    let mut p = Parser { s: &s, i: 0 };
    let v = p.expr()?;
    (p.i == s.len() && v.is_finite()).then_some(v)
}

struct Parser<'a> {
    s: &'a [char],
    i: usize,
}

impl Parser<'_> {
    fn peek(&self) -> Option<char> {
        self.s.get(self.i).copied()
    }

    fn eat(&mut self, c: char) -> bool {
        if self.peek() == Some(c) {
            self.i += 1;
            true
        } else {
            false
        }
    }

    // expr := term (('+'|'-') term)*
    fn expr(&mut self) -> Option<f64> {
        let mut v = self.term()?;
        loop {
            if self.eat('+') {
                v += self.term()?;
            } else if self.eat('-') {
                v -= self.term()?;
            } else {
                return Some(v);
            }
        }
    }

    // term := unary (('*'|'/'|'×'|'÷') unary)*
    fn term(&mut self) -> Option<f64> {
        let mut v = self.unary()?;
        loop {
            if self.eat('*') || self.eat('×') {
                v *= self.unary()?;
            } else if self.eat('/') || self.eat('÷') {
                v /= self.unary()?;
            } else {
                return Some(v);
            }
        }
    }

    // unary := '-' unary | '+' unary | power
    fn unary(&mut self) -> Option<f64> {
        if self.eat('-') {
            return Some(-self.unary()?);
        }
        if self.eat('+') {
            return self.unary();
        }
        self.power()
    }

    // power := atom ('^' unary)?   (right associative: 2^3^2 = 2^9)
    fn power(&mut self) -> Option<f64> {
        let base = self.atom()?;
        if self.eat('^') {
            return Some(base.powf(self.unary()?));
        }
        Some(base)
    }

    fn atom(&mut self) -> Option<f64> {
        if self.eat('(') {
            let v = self.expr()?;
            return self.eat(')').then_some(v);
        }
        let start = self.i;
        while self.peek().is_some_and(|c| c.is_ascii_digit() || c == '.') {
            self.i += 1;
        }
        if self.i == start {
            return None;
        }
        self.s[start..self.i].iter().collect::<String>().parse().ok()
    }
}

#[cfg(test)]
mod tests {
    use super::parse;

    #[test]
    fn works() {
        assert_eq!(parse("40*1.5"), Some(60.0));
        assert_eq!(parse("2^3+1"), Some(9.0));
        assert_eq!(parse("2^3^2"), Some(512.0));
        assert_eq!(parse("(12-4)/2"), Some(4.0));
        assert_eq!(parse("-3^2"), Some(-9.0));
        assert_eq!(parse(" 7 "), Some(7.0));
        assert_eq!(parse("1,5*2"), Some(3.0));
        assert_eq!(parse("2*"), None);
        assert_eq!(parse("1/0"), None);
    }
}

/// A DragValue whose text accepts maths and treats an empty field as 0.
pub fn drag<'a, N: egui::emath::Numeric>(v: &'a mut N) -> egui::DragValue<'a> {
    egui::DragValue::new(v).custom_parser(parse)
}
