interface Greeter {
  greet(name: string): string;
}

class ConsoleGreeter implements Greeter {
  public greet(name: string): string {
    return `Hello, ${name}`;
  }
}

console.log(new ConsoleGreeter().greet("World"));
