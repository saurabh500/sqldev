// odbc_test_config.cpp  –  Read test connection info from environment.

#include "odbc_test_fixture.h"
#include <cstdlib>

// ---------------------------------------------------------------------------
// ODBCTestConfig
// ---------------------------------------------------------------------------
ODBCTestConfig& ODBCTestConfig::Instance() {
    static ODBCTestConfig cfg;
    return cfg;
}

ODBCTestConfig::ODBCTestConfig()
    : dsn_       (GetEnv("ODBC_TEST_DSN"))
    , server_    (GetEnv("ODBC_TEST_SERVER", GetEnv("ODBC_SERVER", "127.0.0.1,1433").c_str()))
    , database_  (GetEnv("ODBC_TEST_DATABASE", GetEnv("ODBC_DATABASE", "tempdb").c_str()))
    , uid_       (GetEnv("ODBC_TEST_UID", GetEnv("ODBC_USER", "sa").c_str()))
    , pwd_       (GetEnv("ODBC_TEST_PWD", GetEnv("ODBC_PASSWORD").c_str()))
    , driver_    (GetEnv("ODBC_TEST_DRIVER",
                        GetEnv("ODBC_DRIVER", "ODBC Driver 18 for SQL Server").c_str()))
    , connstr_   (GetEnv("ODBC_TEST_CONNSTR", GetEnv("ODBC_CONNECTION_STRING").c_str()))
    , trust_cert_(GetEnv("ODBC_TEST_TRUST_CERT",  "Yes"))
    , encrypt_   (GetEnv("ODBC_TEST_ENCRYPT"))
{}

std::string ODBCTestConfig::GetEnv(const char* name, const char* fallback) {
#ifdef _WIN32
    // Use _dupenv_s to avoid MSVC deprecation warning for getenv.
    char* buf = nullptr;
    size_t len = 0;
    if (_dupenv_s(&buf, &len, name) == 0 && buf != nullptr) {
        std::string val(buf);
        free(buf);
        return val;
    }
    return fallback ? fallback : "";
#else
    const char* val = std::getenv(name);
    return (val && val[0]) ? std::string(val)
                           : (fallback ? std::string(fallback) : std::string());
#endif
}
