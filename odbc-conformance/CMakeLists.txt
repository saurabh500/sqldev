cmake_minimum_required(VERSION 3.16)

project(sqldev_odbc_conformance VERSION 0.1.0 LANGUAGES C)

include(CTest)
find_package(ODBC REQUIRED)

add_executable(odbc-conformance src/conformance.c)
target_compile_features(odbc-conformance PRIVATE c_std_99)
target_link_libraries(odbc-conformance PRIVATE ODBC::ODBC)

if(CMAKE_C_COMPILER_ID MATCHES "GNU|Clang")
  target_compile_options(odbc-conformance PRIVATE -Wall -Wextra -Wpedantic -Werror)
elseif(MSVC)
  target_compile_options(odbc-conformance PRIVATE /W4 /WX)
endif()

set(CONFORMANCE_TESTS
    connection
    statements
    parameters
    transactions
    metadata
    diagnostics
    unicode)

foreach(test_name IN LISTS CONFORMANCE_TESTS)
  add_test(NAME odbc.${test_name}
           COMMAND odbc-conformance ${test_name})
  set_tests_properties(odbc.${test_name} PROPERTIES
    TIMEOUT 30
    RUN_SERIAL TRUE)
endforeach()
