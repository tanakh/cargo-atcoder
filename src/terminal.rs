use std::{fmt, io};
use termcolor::{BufferedStandardStream, Color, ColorChoice, ColorSpec, WriteColor};

pub(crate) fn stderr() -> BufferedStandardStream {
    BufferedStandardStream::stderr(if atty::is(atty::Stream::Stderr) {
        ColorChoice::Auto
    } else {
        ColorChoice::Never
    })
}

pub(crate) trait WriteColorExt: WriteColor {
    fn warn(&mut self, message: impl fmt::Display) -> io::Result<()> {
        self.set_color(
            ColorSpec::new()
                .set_fg(Some(Color::Yellow))
                .set_bold(true)
                .set_reset(false),
        )?;
        self.write_all(b"warning:")?;
        self.reset()?;
        writeln!(self, " {}", message)?;
        self.flush()
    }

    fn status_with_color(
        &mut self,
        status: impl fmt::Display,
        message: impl fmt::Display,
        color: Color,
    ) -> io::Result<()> {
        self.set_color(
            ColorSpec::new()
                .set_fg(Some(color))
                .set_bold(true)
                .set_reset(false),
        )?;
        write!(self, "{:>12}", status)?;
        self.reset()?;
        writeln!(self, " {}", message)?;
        self.flush()
    }
}

impl<W: WriteColor> WriteColorExt for W {}
