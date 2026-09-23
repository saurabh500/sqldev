cmake_minimum_required(VERSION 3.15)
project(rust_odbc_parity LANGUAGES CXX)

# C++17 is the minimum for GoogleTest (v1.17+).
set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)

find_package(Threads QUIET)
if(NOT Threads_FOUND)
    set(CMAKE_THREAD_LIBS_INIT "-lpthread")
    set(THREADS_PREFER_PTHREAD_FLAG ON)
endif()

# ---------------------------------------------------------------------------
# Google Test via FetchContent — self-contained, no submodule/system package
# required. Pinned to the same revision used when these suites were authored.
# ---------------------------------------------------------------------------
include(FetchContent)
FetchContent_Declare(
    googletest
    GIT_REPOSITORY https://github.com/google/googletest.git
    GIT_TAG 52eb8108c5bdec04579160ae17225d66034bd723 # v1.17.0
)
set(gtest_force_shared_crt ON CACHE BOOL "" FORCE)
FetchContent_MakeAvailable(googletest)

# ---------------------------------------------------------------------------
# ODBC (unixODBC on Linux/macOS, odbc32/odbccp32 on Windows)
# ---------------------------------------------------------------------------
if(WIN32)
    set(ODBC_LIBRARIES odbc32 odbccp32)
else()
    find_library(ODBC_LIB NAMES odbc odbc2 iodbc)
    if(NOT ODBC_LIB)
        message(FATAL_ERROR "Could not find an ODBC library (unixODBC or iODBC). "
                            "Install unixodbc-dev or equivalent.")
    endif()
    find_library(ODBCINST_LIB NAMES odbcinst iodbcinst)
    set(ODBC_LIBRARIES ${ODBC_LIB})
    if(ODBCINST_LIB)
        list(APPEND ODBC_LIBRARIES ${ODBCINST_LIB})
    endif()

    find_path(ODBC_INCLUDE_DIR sql.h
        PATHS /opt/homebrew/include /usr/local/include /usr/include
    )
    if(NOT ODBC_INCLUDE_DIR)
        message(FATAL_ERROR "Could not find sql.h. Ensure unixODBC headers are installed.")
    endif()
endif()

# ---------------------------------------------------------------------------
# odbc_test_lib — shared fixture (ported unchanged from mssql-rs/mssql-odbc's
# e2e harness: include/odbc_test_fixture.h + lib/odbc_test_*.cpp)
# ---------------------------------------------------------------------------
add_library(odbc_test_lib STATIC
    lib/odbc_test_fixture.cpp
    lib/odbc_test_utils.cpp
    lib/odbc_test_config.cpp
)
target_include_directories(odbc_test_lib PUBLIC
    ${CMAKE_CURRENT_SOURCE_DIR}/include
)
if(ODBC_INCLUDE_DIR)
    target_include_directories(odbc_test_lib PUBLIC ${ODBC_INCLUDE_DIR})
endif()
target_link_libraries(odbc_test_lib PUBLIC gtest ${ODBC_LIBRARIES})

# ---------------------------------------------------------------------------
# Helper: add a test executable under tests/, matching the parent project's
# add_odbc_test() convention so these suites stay drop-in compatible.
# ---------------------------------------------------------------------------
function(add_odbc_test TEST_NAME)
    add_executable(${TEST_NAME} ${ARGN})
    target_link_libraries(${TEST_NAME} PRIVATE odbc_test_lib gtest_main)
    add_test(NAME ${TEST_NAME} COMMAND ${TEST_NAME})
endfunction()

enable_testing()

add_odbc_test(numeric_parser_conversion_test tests/numeric_parser_conversion_test.cpp)
add_odbc_test(interval_conversion_test       tests/interval_conversion_test.cpp)
