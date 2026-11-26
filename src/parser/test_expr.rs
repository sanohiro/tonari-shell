use super::*;

// The grammar has a fixed set of operators; store their spelling in the AST.
macro_rules! operator {
    ($word:expr, $( $op:literal ),+ $(,)?) => {
        match $word { $( Some($op) => Some($op), )+ _ => None }
    };
}

impl Reader {
    pub(super) fn test_or(&mut self) -> Result<Expr, Error> {
        let first = self.test_and()?;
        if !matches!(self.peek(), Some(Token::Or)) {
            return Ok(first);
        }
        let mut terms = vec![first];
        while matches!(self.peek(), Some(Token::Or)) {
            self.tokens.pop_front();
            self.newlines();
            terms.push(self.test_and()?);
        }
        Ok(Expr::Or(terms))
    }

    fn test_and(&mut self) -> Result<Expr, Error> {
        let first = self.test_atom()?;
        self.newlines();
        if !matches!(self.peek(), Some(Token::And)) {
            return Ok(first);
        }
        let mut terms = vec![first];
        while matches!(self.peek(), Some(Token::And)) {
            self.tokens.pop_front();
            self.newlines();
            terms.push(self.test_atom()?);
            self.newlines();
        }
        Ok(Expr::And(terms))
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
            return Ok(Expr::Not(Box::new(self.nested(Self::test_atom)?)));
        }
        if matches!(self.peek(), Some(Token::LeftParen)) {
            self.tokens.pop_front();
            let e = self.nested(Self::test_or)?;
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
        if let Some(op) = operator!(
            self.keyword(),
            "-a",
            "-b",
            "-c",
            "-d",
            "-e",
            "-f",
            "-g",
            "-h",
            "-k",
            "-n",
            "-o",
            "-p",
            "-r",
            "-s",
            "-t",
            "-u",
            "-v",
            "-w",
            "-x",
            "-z",
            "-G",
            "-L",
            "-N",
            "-O",
            "-R",
            "-S"
        ) {
            self.tokens.pop_front();
            return Ok(Expr::Unary(op, self.test_word()?));
        }
        let left = self.test_word()?;
        let op = match self.peek() {
            Some(Token::Redirect(0, Target::Read)) => "<",
            Some(Token::Redirect(1, Target::Write)) => ">",
            _ => match operator!(
                self.keyword(),
                "=",
                "==",
                "!=",
                "=~",
                "-eq",
                "-ne",
                "-lt",
                "-gt",
                "-le",
                "-ge",
                "-ef",
                "-nt",
                "-ot"
            ) {
                Some(op) => op,
                None => return Ok(Expr::Word(left)),
            },
        };
        self.tokens.pop_front();
        Ok(Expr::binary(left, op, self.test_word()?))
    }
}
