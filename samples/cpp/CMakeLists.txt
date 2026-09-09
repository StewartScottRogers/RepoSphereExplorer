cmake_minimum_required(VERSION 3.24)

project(linalg
        VERSION 2.0.0
        DESCRIPTION "A small dense matrix template with checked dimensions."
        LANGUAGES CXX)

set(CMAKE_CXX_STANDARD 20)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_CXX_EXTENSIONS OFF)
set(CMAKE_EXPORT_COMPILE_COMMANDS ON)

add_executable(matrix src/matrix.cpp)
target_include_directories(matrix PRIVATE include)

if(CMAKE_CXX_COMPILER_ID MATCHES "GNU|Clang")
  target_compile_options(matrix PRIVATE
    -Wall -Wextra -Wpedantic -Wshadow -Wconversion -Wold-style-cast -Werror)
endif()

include(CTest)

# The program asserts its own invariants and returns non-zero when one of
# them fails, so running it is the smoke test. Splitting the template out
# of the translation unit that demonstrates it would be a bigger change
# than this fixture is for.
add_test(NAME smoke COMMAND matrix)
set_tests_properties(smoke PROPERTIES TIMEOUT 10)
