cmake_minimum_required(VERSION 3.21)

project(sqldev_odbc_conformance VERSION 0.1.0 LANGUAGES CXX)

include(CTest)
find_package(ODBC REQUIRED)
find_package(GTest REQUIRED)
find_path(MSODBCSQL_INCLUDE_DIR msodbcsql.h
  PATHS /opt/microsoft/msodbcsql18/include
  REQUIRED)

add_executable(odbc-conformance
  src/conformance.cpp
  ../rust-odbc-parity/lib/odbc_test_config.cpp
  ../rust-odbc-parity/lib/odbc_test_fixture.cpp
  ../rust-odbc-parity/lib/odbc_test_utils.cpp
  ../rust-odbc-parity/tests/interval_conversion_test.cpp
  ../rust-odbc-parity/tests/numeric_parser_conversion_test.cpp)
target_compile_features(odbc-conformance PRIVATE cxx_std_17)
target_link_libraries(odbc-conformance PRIVATE ODBC::ODBC GTest::gtest_main)
target_include_directories(odbc-conformance SYSTEM PRIVATE "${MSODBCSQL_INCLUDE_DIR}")
target_include_directories(odbc-conformance PRIVATE ../rust-odbc-parity/include)

if(CMAKE_CXX_COMPILER_ID MATCHES "GNU|Clang")
  target_compile_options(odbc-conformance PRIVATE -Wall -Wextra -Wpedantic -Werror)
elseif(MSVC)
  target_compile_options(odbc-conformance PRIVATE /W4 /WX)
endif()

include(GoogleTest)
gtest_discover_tests(odbc-conformance
  DISCOVERY_TIMEOUT 30
  NO_PRETTY_VALUES
  XML_OUTPUT_DIR "${CMAKE_CURRENT_BINARY_DIR}/test-results"
  PROPERTIES TIMEOUT 30 RUN_SERIAL TRUE)
