import 'dart:core';

class Greeter {
  final String name;

  Greeter(this.name);

  @override
  String toString() => 'Hello, $name!';
}

void main() {
  final greeter = Greeter('world');
  print(greeter);
}
