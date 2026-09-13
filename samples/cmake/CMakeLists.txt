# csvstats: summary statistics for a column of readings.

cmake_minimum_required(VERSION 3.25)

project(csvstats
  VERSION 1.0.3
  DESCRIPTION "Summary statistics for a column of readings"
  HOMEPAGE_URL "https://example.com/floor/csvstats"
  LANGUAGES CXX C
)

set(CMAKE_CXX_STANDARD 20)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_EXPORT_COMPILE_COMMANDS ON)

option(CSVSTATS_BUILD_TESTS "Build the test suite" ON)
option(CSVSTATS_WITH_ZLIB "Read gzipped readings" OFF)
option(CSVSTATS_STRICT "Treat warnings as errors" OFF)

find_package(Threads REQUIRED)
find_package(fmt 10.2 CONFIG REQUIRED)

if(CSVSTATS_WITH_ZLIB)
  find_package(ZLIB REQUIRED)
endif()

add_library(csvstats_core STATIC
  src/column.cc
  src/reader.cc
  src/summary.cc
)

target_include_directories(csvstats_core
  PUBLIC $<BUILD_INTERFACE:${CMAKE_CURRENT_SOURCE_DIR}/include>
)

target_link_libraries(csvstats_core
  PUBLIC fmt::fmt
  PRIVATE Threads::Threads
)

if(CSVSTATS_STRICT)
  target_compile_options(csvstats_core PRIVATE -Wall -Wextra -Werror)
endif()

add_executable(csvstats src/main.cc)
target_link_libraries(csvstats PRIVATE csvstats_core)

add_subdirectory(tools)
add_subdirectory(docs)

if(CSVSTATS_BUILD_TESTS)
  enable_testing()
  add_subdirectory(test)

  add_executable(column_test test/column_test.cc)
  target_link_libraries(column_test PRIVATE csvstats_core)

  add_test(NAME column COMMAND column_test)
  add_test(NAME summary COMMAND column_test --gtest_filter=Summary.*)
endif()

install(TARGETS csvstats csvstats_core
  RUNTIME DESTINATION bin
  ARCHIVE DESTINATION lib
)
