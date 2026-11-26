use super::*;

impl Reader {
    pub(super) fn test_or(&mut self) -> Result<Expr, Error> {
        let mut result = self.test_and()?;
        while matches!(self.peek(), Some(Token::Or)) {
            self.tokens.pop_front();
            self.newlines();
            result = Expr::Or(Box::new(result), Box::new(self.test_and()?));
        }
        Ok(result)
    }

    fn test_and(&mut self) -> Result<Expr, Error> {
        let mut result = self.test_atom()?;
        self.newlines();
        while matches!(self.peek(), Some(Token::And)) {
            self.tokens.pop_front();
            self.newlines();
            result = Expr::And(Box::new(result), Box::new(self.test_atom()?));
            self.newlines();
        }
        Ok(result)
    }

    fn test_word(&mut self) -> Result<Word, Error> {
        if self.keyword() == Some("]]") {
            return Err(Error::Invalid("missing conditional operand"));
        }
        match self.operand() {
            Some(Token::Word(word)) => Ok(word.word),
            None => Err(Error::Incomplete(Incomplete::Message(
                "unfinished conditional expression",
            ))),
            _ => Err(Error::Invalid("expected a conditional operand")),
        }
    }

    fn test_atom(&mut self) -> Result<Expr, Error> {
        self.newlines();
        if self.keyword() == Some("!") {
            self.tokens.pop_front();
            return Ok(Expr::Not(Box::new(self.test_atom()?)));
        }
        if matches!(self.peek(), Some(Token::LeftParen)) {
            self.tokens.pop_front();
            let e = self.test_or()?;
            match self.tokens.pop_front() {
                Some(Token::RightParen) => return Ok(e),
                None => {
                    return Err(Error::Incomplete(Incomplete::Message(
                        "unclosed conditional grouping",
                    )));
                }
                _ => return Err(Error::Invalid("expected ')' in conditional expression")),
            }
        }
        let unary = self.keyword().is_some_and(|op| {
            matches!(
                op,
                "-a" | "-b"
                    | "-c"
                    | "-d"
                    | "-e"
                    | "-f"
                    | "-g"
                    | "-h"
                    | "-k"
                    | "-n"
                    | "-o"
                    | "-p"
                    | "-r"
                    | "-s"
                    | "-t"
                    | "-u"
                    | "-v"
                    | "-w"
                    | "-x"
                    | "-z"
                    | "-G"
                    | "-L"
                    | "-N"
                    | "-O"
                    | "-R"
                    | "-S"
            )
        });
        if unary {
            let op = self.keyword().unwrap().to_owned();
            self.tokens.pop_front();
            return Ok(Expr::Unary(op, self.test_word()?));
        }
        let left = self.test_word()?;
        let op = match self.peek() {
            Some(Token::Redirect(0, Target::Read)) => "<",
            Some(Token::Redirect(1, Target::Write)) => ">",
            _ => match self.keyword() {
                Some(
                    op @ ("=" | "==" | "!=" | "=~" | "-eq" | "-ne" | "-lt" | "-gt" | "-le" | "-ge"
                    | "-ef" | "-nt" | "-ot"),
                ) => op,
                _ => return Ok(Expr::Word(left)),
            },
        }
        .to_owned();
        self.tokens.pop_front();
        Ok(Expr::binary(left, op, self.test_word()?))
    }
}
