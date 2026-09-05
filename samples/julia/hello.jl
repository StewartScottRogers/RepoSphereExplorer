using Printf

function greet(name::String)
    counts = Dict{String, Int}("greetings" => 1)
    @printf("Hello, %s! (%d)\n", name, counts["greetings"])
end

greet("World")
