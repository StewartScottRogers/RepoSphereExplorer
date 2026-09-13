# csvstats: summary statistics for a column of readings.

project('csvstats', 'cpp', 'c',
  version : '1.0.3',
  license : 'MIT',
  meson_version : '>=1.3.0',
  default_options : [
    'cpp_std=c++20',
    'warning_level=3',
    'werror=false',
  ],
)

build_tests = get_option('tests')
with_zlib = get_option('zlib')
prefix = get_option('prefix')

threads_dep = dependency('threads')
fmt_dep = dependency('fmt', version : '>=10.2', required : true)
zlib_dep = dependency('zlib', required : with_zlib)

inc = include_directories('include')

csvstats_lib = library('csvstats',
  'src/column.cc',
  'src/reader.cc',
  'src/summary.cc',
  include_directories : inc,
  dependencies : [fmt_dep, threads_dep, zlib_dep],
  install : true,
)

csvstats_dep = declare_dependency(
  link_with : csvstats_lib,
  include_directories : inc,
)

csvstats_exe = executable('csvstats',
  'src/main.cc',
  dependencies : [csvstats_dep],
  install : true,
)

if build_tests
  column_test = executable('column_test',
    'test/column_test.cc',
    dependencies : [csvstats_dep],
  )

  test('column', column_test)
  test('summary', column_test, args : ['--filter=summary'], timeout : 30)
endif

subdir('tools')
subdir('docs')

install_headers('include/csvstats/column.h', subdir : 'csvstats')
