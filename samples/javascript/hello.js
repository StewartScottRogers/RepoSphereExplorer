function greet(name) {
  return `Hello, ${name}!`;
}

function main() {
  console.log(greet("World"));
}

main();

module.exports = { greet };
