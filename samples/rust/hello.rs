use std::fmt;

/// A greeting's name, aliased to exercise a `type` alias.
type Name = String;

/// How loudly to greet.
enum Volume {
    Quiet,
    Loud,
}

struct Greeting {
    name: Name,
    volume: Volume,
}

impl Greeting {
    fn new(name: &str) -> Self {
        Greeting {
            name: name.to_string(),
            volume: Volume::Quiet,
        }
    }

    fn parse(text: &str) -> Self {
        match text.split_once(',') {
            Some((name, _rest)) => Greeting::new(name.trim()),
            None => Greeting::new(text.trim()),
        }
    }

    fn suffix(&self) -> &'static str {
        match self.volume {
            Volume::Quiet => ".",
            Volume::Loud => "!",
        }
    }
}

impl fmt::Display for Greeting {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "Hello, {}{}", self.name, self.suffix())
    }
}

fn main() {
    let mut greeting = Greeting::parse("World, and everyone else");
    greeting.volume = Volume::Loud;
    println!("{}", greeting);
}
