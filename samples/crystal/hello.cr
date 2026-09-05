class Greeter
  property name : String

  def initialize(@name : String)
  end

  def greet
    puts "Hello, #{@name}!"
  end
end

greeter = Greeter.new("world")
greeter.greet
