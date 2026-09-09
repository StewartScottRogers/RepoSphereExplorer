cmake_minimum_required(VERSION 3.24)

project(ringbuffer
        VERSION 1.0.2
        DESCRIPTION "A lock-free single-producer ring buffer."
        LANGUAGES C)

set(CMAKE_C_STANDARD 17)
set(CMAKE_C_STANDARD_REQUIRED ON)
set(CMAKE_C_EXTENSIONS OFF)
set(CMAKE_EXPORT_COMPILE_COMMANDS ON)

add_executable(ringbuffer src/ringbuffer.c)
target_include_directories(ringbuffer PRIVATE include)

if(CMAKE_C_COMPILER_ID MATCHES "GNU|Clang")
  target_compile_options(ringbuffer PRIVATE
    -Wall -Wextra -Wpedantic -Wshadow -Wconversion -Werror)
endif()

include(CTest)

# The demo exercises push, pop and drain and returns non-zero if any of
# them misbehaves, so running it *is* the smoke test. A separate test
# binary would need the functions in their own translation unit, which is
# a bigger change than this fixture is for.
add_test(NAME smoke COMMAND ringbuffer)
set_tests_properties(smoke PROPERTIES TIMEOUT 10)
