use std::fmt;

struct Greeting {
    name: String,
}

impl Greeting {
    fn new(name: &str) -> Self {
        Greeting {
            name: name.to_string(),
        }
    }
}

impl fmt::Display for Greeting {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "Hello, {}", self.name)
    }
}

fn main() {
    let mut greeting = Greeting::new("World");
    greeting.name = String::from("World");
    println!("{}", greeting);
}
