#include <gtest/gtest.h>
#include <sql.h>
#include <sqlext.h>

#include "datatype_samples.h"

#include <algorithm>
#include <array>
#include <chrono>
#include <cstddef>
#include <cstdlib>
#include <cstring>
#include <iostream>
#include <iterator>
#include <random>
#include <sstream>
#include <string>
#include <tuple>
#include <vector>

namespace {

SQLCHAR* sql_text(const char* text)
{
    // ODBC's input-only text arguments predate const-correct interfaces.
    return reinterpret_cast<SQLCHAR*>(const_cast<char*>(text));
}

std::string diagnostics(SQLSMALLINT type, SQLHANDLE handle)
{
    std::ostringstream result;
    for (SQLSMALLINT record = 1; ; ++record) {
        SQLCHAR state[SQL_SQLSTATE_SIZE + 1]{};
        SQLCHAR message[1024]{};
        SQLINTEGER native_error = 0;
        SQLSMALLINT length = 0;
        const auto rc = SQLGetDiagRec(type, handle, record, state, &native_error,
                                      message, sizeof(message), &length);
        if (rc == SQL_NO_DATA) {
            break;
        }
        if (!SQL_SUCCEEDED(rc)) {
            result << "SQLGetDiagRec failed: " << rc;
            break;
        }
        result << "\n[" << state << "] (" << native_error << ") " << message;
    }
    return result.str();
}

testing::AssertionResult odbc_ok(SQLRETURN rc, SQLSMALLINT type, SQLHANDLE handle)
{
    if (SQL_SUCCEEDED(rc)) {
        return testing::AssertionSuccess();
    }
    return testing::AssertionFailure()
           << "SQLRETURN " << rc << diagnostics(type, handle);
}

#define ASSERT_ODBC(call, type, handle) ASSERT_TRUE(odbc_ok((call), (type), (handle)))
#define EXPECT_ODBC(call, type, handle) EXPECT_TRUE(odbc_ok((call), (type), (handle)))

template<SQLSMALLINT Type>
class Handle {
public:
    SQLHANDLE value = SQL_NULL_HANDLE;

    Handle() = default;
    Handle(const Handle&) = delete;
    Handle& operator=(const Handle&) = delete;
    ~Handle() { reset(); }

    void reset()
    {
        if (value != SQL_NULL_HANDLE) {
            EXPECT_ODBC(SQLFreeHandle(Type, value), Type, value);
            value = SQL_NULL_HANDLE;
        }
    }
};

const char* env_or_default(const char* name, const char* fallback)
{
    const auto* value = std::getenv(name);
    return value != nullptr && *value != '\0' ? value : fallback;
}

std::string attribute(const char* name, const char* value)
{
    std::string result = std::string(name) + "={";
    for (; *value != '\0'; ++value) {
        result += *value;
        if (*value == '}') {
            result += '}';
        }
    }
    return result + "};";
}

class OdbcConformance : public testing::Test {
protected:
    Handle<SQL_HANDLE_ENV> env;
    Handle<SQL_HANDLE_DBC> dbc;
    Handle<SQL_HANDLE_STMT> stmt;
    bool connected = false;
    bool manual_commit = false;
    std::string metadata_table;

    void SetUp() override
    {
        ASSERT_EQ(SQL_SUCCESS,
                  SQLAllocHandle(SQL_HANDLE_ENV, SQL_NULL_HANDLE, &env.value));
        ASSERT_ODBC(SQLSetEnvAttr(env.value, SQL_ATTR_ODBC_VERSION,
                                 reinterpret_cast<SQLPOINTER>(SQL_OV_ODBC3), 0),
                    SQL_HANDLE_ENV, env.value);
        ASSERT_ODBC(SQLAllocHandle(SQL_HANDLE_DBC, env.value, &dbc.value),
                    SQL_HANDLE_ENV, env.value);
        ASSERT_ODBC(SQLSetConnectAttr(dbc.value, SQL_LOGIN_TIMEOUT,
                                     reinterpret_cast<SQLPOINTER>(10), 0),
                    SQL_HANDLE_DBC, dbc.value);

        std::string settings = env_or_default("ODBC_CONNECTION_STRING", "");
        if (settings.empty()) {
            settings = attribute("DRIVER", env_or_default(
                            "ODBC_DRIVER", "ODBC Driver 18 for SQL Server")) +
                       attribute("SERVER", env_or_default("ODBC_SERVER", "127.0.0.1,1433")) +
                       attribute("UID", env_or_default("ODBC_USER", "sa")) +
                       attribute("PWD", env_or_default("ODBC_PASSWORD", "")) +
                       attribute("DATABASE", env_or_default("ODBC_DATABASE", "tempdb")) +
                       "Encrypt=yes;TrustServerCertificate=yes;";
        }
        ASSERT_ODBC(SQLDriverConnect(dbc.value, nullptr, sql_text(settings.c_str()),
                                    SQL_NTS, nullptr, 0, nullptr, SQL_DRIVER_NOPROMPT),
                    SQL_HANDLE_DBC, dbc.value);
        connected = true;
        ASSERT_ODBC(SQLAllocHandle(SQL_HANDLE_STMT, dbc.value, &stmt.value),
                    SQL_HANDLE_DBC, dbc.value);
    }

    void TearDown() override
    {
        // Bound buffers belong to the test body and are no longer alive here.
        stmt.reset();
        if (connected) {
            if (manual_commit) {
                EXPECT_ODBC(SQLEndTran(SQL_HANDLE_DBC, dbc.value, SQL_ROLLBACK),
                            SQL_HANDLE_DBC, dbc.value);
                EXPECT_ODBC(SQLSetConnectAttr(dbc.value, SQL_ATTR_AUTOCOMMIT,
                            reinterpret_cast<SQLPOINTER>(SQL_AUTOCOMMIT_ON), 0),
                            SQL_HANDLE_DBC, dbc.value);
            }
            if (!metadata_table.empty()) {
                Handle<SQL_HANDLE_STMT> cleanup;
                const auto rc = SQLAllocHandle(SQL_HANDLE_STMT, dbc.value, &cleanup.value);
                EXPECT_ODBC(rc, SQL_HANDLE_DBC, dbc.value);
                if (SQL_SUCCEEDED(rc)) {
                    const auto sql = "DROP TABLE IF EXISTS dbo." + metadata_table;
                    EXPECT_ODBC(SQLExecDirect(cleanup.value, sql_text(sql.c_str()), SQL_NTS),
                                SQL_HANDLE_STMT, cleanup.value);
                }
            }
            EXPECT_ODBC(SQLDisconnect(dbc.value), SQL_HANDLE_DBC, dbc.value);
        }
        dbc.reset();
        env.reset();
    }

    void execute(const char* sql)
    {
        ASSERT_ODBC(SQLExecDirect(stmt.value, sql_text(sql), SQL_NTS),
                    SQL_HANDLE_STMT, stmt.value);
    }

    void close_cursor()
    {
        ASSERT_ODBC(SQLFreeStmt(stmt.value, SQL_CLOSE), SQL_HANDLE_STMT, stmt.value);
    }

    void expect_scalar(const char* sql, SQLINTEGER expected)
    {
        ASSERT_NO_FATAL_FAILURE(execute(sql));
        ASSERT_ODBC(SQLFetch(stmt.value), SQL_HANDLE_STMT, stmt.value);
        SQLINTEGER actual = 0;
        SQLLEN indicator = 0;
        ASSERT_ODBC(SQLGetData(stmt.value, 1, SQL_C_SLONG, &actual, sizeof(actual),
                               &indicator), SQL_HANDLE_STMT, stmt.value);
        ASSERT_NE(SQL_NULL_DATA, indicator);
        EXPECT_EQ(expected, actual);
        EXPECT_EQ(SQL_NO_DATA, SQLFetch(stmt.value));
        ASSERT_NO_FATAL_FAILURE(close_cursor());
    }
};

TEST_F(OdbcConformance, Connection)
{
    SQLCHAR driver[256]{};
    SQLCHAR version[256]{};
    SQLCHAR manager[256]{};
    SQLSMALLINT length = 0;
    ASSERT_ODBC(SQLGetInfo(dbc.value, SQL_DRIVER_NAME, driver, sizeof(driver), &length),
                SQL_HANDLE_DBC, dbc.value);
    EXPECT_NE(std::string::npos,
              std::string(reinterpret_cast<char*>(driver)).find("msodbcsql"));
    ASSERT_ODBC(SQLGetInfo(dbc.value, SQL_DRIVER_VER, version, sizeof(version), &length),
                SQL_HANDLE_DBC, dbc.value);
    EXPECT_NE('\0', version[0]);
    ASSERT_ODBC(SQLGetInfo(dbc.value, SQL_DM_VER, manager, sizeof(manager), &length),
                SQL_HANDLE_DBC, dbc.value);
    EXPECT_NE('\0', manager[0]);
    std::cout << "driver=" << driver << " driver_version=" << version
              << " driver_manager_version=" << manager << '\n';
}

TEST_F(OdbcConformance, Statements)
{
    ASSERT_NO_FATAL_FAILURE(execute("SELECT CAST(42 AS INT) AS answer"));
    SQLSMALLINT columns = 0;
    ASSERT_ODBC(SQLNumResultCols(stmt.value, &columns), SQL_HANDLE_STMT, stmt.value);
    EXPECT_EQ(1, columns);

    SQLCHAR name[64]{};
    SQLSMALLINT length = 0, type = 0, digits = 0, nullable = 0;
    SQLULEN size = 0;
    ASSERT_ODBC(SQLDescribeCol(stmt.value, 1, name, sizeof(name), &length, &type,
                              &size, &digits, &nullable), SQL_HANDLE_STMT, stmt.value);
    EXPECT_STREQ("answer", reinterpret_cast<char*>(name));
    EXPECT_EQ(SQL_INTEGER, type);
    ASSERT_ODBC(SQLFetch(stmt.value), SQL_HANDLE_STMT, stmt.value);
    SQLINTEGER answer = 0;
    SQLLEN indicator = 0;
    ASSERT_ODBC(SQLGetData(stmt.value, 1, SQL_C_SLONG, &answer, sizeof(answer),
                          &indicator), SQL_HANDLE_STMT, stmt.value);
    ASSERT_NE(SQL_NULL_DATA, indicator);
    EXPECT_EQ(42, answer);
    EXPECT_EQ(SQL_NO_DATA, SQLFetch(stmt.value));
}

TEST_F(OdbcConformance, Parameters)
{
    SQLINTEGER left = 20, right = 22, result = 0;
    SQLLEN left_indicator = 0, right_indicator = 0, result_indicator = 0;
    ASSERT_ODBC(SQLPrepare(stmt.value, sql_text("SELECT CAST(? + ? AS INT)"), SQL_NTS),
                SQL_HANDLE_STMT, stmt.value);
    ASSERT_ODBC(SQLBindParameter(stmt.value, 1, SQL_PARAM_INPUT, SQL_C_SLONG,
                                SQL_INTEGER, 0, 0, &left, 0, &left_indicator),
                SQL_HANDLE_STMT, stmt.value);
    ASSERT_ODBC(SQLBindParameter(stmt.value, 2, SQL_PARAM_INPUT, SQL_C_SLONG,
                                SQL_INTEGER, 0, 0, &right, 0, &right_indicator),
                SQL_HANDLE_STMT, stmt.value);
    ASSERT_ODBC(SQLExecute(stmt.value), SQL_HANDLE_STMT, stmt.value);
    ASSERT_ODBC(SQLFetch(stmt.value), SQL_HANDLE_STMT, stmt.value);
    ASSERT_ODBC(SQLGetData(stmt.value, 1, SQL_C_SLONG, &result, sizeof(result),
                          &result_indicator), SQL_HANDLE_STMT, stmt.value);
    ASSERT_NE(SQL_NULL_DATA, result_indicator);
    EXPECT_EQ(42, result);
    EXPECT_EQ(SQL_NO_DATA, SQLFetch(stmt.value));
}

TEST_F(OdbcConformance, Transactions)
{
    ASSERT_NO_FATAL_FAILURE(execute("CREATE TABLE #odbc_transactions (value INT NOT NULL)"));
    ASSERT_NO_FATAL_FAILURE(close_cursor());
    ASSERT_ODBC(SQLSetConnectAttr(dbc.value, SQL_ATTR_AUTOCOMMIT,
                                 reinterpret_cast<SQLPOINTER>(SQL_AUTOCOMMIT_OFF), 0),
                SQL_HANDLE_DBC, dbc.value);
    manual_commit = true;
    ASSERT_NO_FATAL_FAILURE(execute("INSERT INTO #odbc_transactions VALUES (1)"));
    ASSERT_NO_FATAL_FAILURE(close_cursor());
    ASSERT_ODBC(SQLEndTran(SQL_HANDLE_DBC, dbc.value, SQL_ROLLBACK),
                SQL_HANDLE_DBC, dbc.value);
    ASSERT_NO_FATAL_FAILURE(expect_scalar("SELECT COUNT(*) FROM #odbc_transactions", 0));

    ASSERT_NO_FATAL_FAILURE(execute("INSERT INTO #odbc_transactions VALUES (2)"));
    ASSERT_NO_FATAL_FAILURE(close_cursor());
    ASSERT_ODBC(SQLEndTran(SQL_HANDLE_DBC, dbc.value, SQL_COMMIT),
                SQL_HANDLE_DBC, dbc.value);
    ASSERT_NO_FATAL_FAILURE(expect_scalar("SELECT COUNT(*) FROM #odbc_transactions", 1));
}

TEST_F(OdbcConformance, Metadata)
{
    // Catalog functions need a visible table; give each run its own object.
    const auto suffix = std::chrono::steady_clock::now().time_since_epoch().count();
    metadata_table = "odbc_conf_metadata_" + std::to_string(std::random_device{}()) +
                     "_" + std::to_string(suffix);
    const auto sql = "CREATE TABLE dbo." + metadata_table +
                     " (id INT NOT NULL, label NVARCHAR(50) NULL)";
    ASSERT_NO_FATAL_FAILURE(execute(sql.c_str()));
    ASSERT_NO_FATAL_FAILURE(close_cursor());
    ASSERT_ODBC(SQLTables(stmt.value, nullptr, 0, sql_text("dbo"), SQL_NTS,
                         sql_text(metadata_table.c_str()), SQL_NTS,
                         sql_text("TABLE"), SQL_NTS), SQL_HANDLE_STMT, stmt.value);
    bool found_table = false;
    for (;;) {
        const auto rc = SQLFetch(stmt.value);
        if (rc == SQL_NO_DATA) {
            break;
        }
        ASSERT_ODBC(rc, SQL_HANDLE_STMT, stmt.value);
        SQLCHAR name[128]{};
        SQLLEN indicator = 0;
        ASSERT_ODBC(SQLGetData(stmt.value, 3, SQL_C_CHAR, name, sizeof(name),
                              &indicator), SQL_HANDLE_STMT, stmt.value);
        ASSERT_NE(SQL_NULL_DATA, indicator);
        found_table |= metadata_table == reinterpret_cast<char*>(name);
    }
    EXPECT_TRUE(found_table);
    ASSERT_NO_FATAL_FAILURE(close_cursor());
    ASSERT_ODBC(SQLColumns(stmt.value, nullptr, 0, sql_text("dbo"), SQL_NTS,
                          sql_text(metadata_table.c_str()), SQL_NTS, nullptr, 0),
                SQL_HANDLE_STMT, stmt.value);
    bool found_id = false, found_label = false;
    for (;;) {
        const auto rc = SQLFetch(stmt.value);
        if (rc == SQL_NO_DATA) {
            break;
        }
        ASSERT_ODBC(rc, SQL_HANDLE_STMT, stmt.value);
        SQLCHAR name[128]{};
        SQLLEN indicator = 0;
        ASSERT_ODBC(SQLGetData(stmt.value, 4, SQL_C_CHAR, name, sizeof(name),
                              &indicator), SQL_HANDLE_STMT, stmt.value);
        ASSERT_NE(SQL_NULL_DATA, indicator);
        found_id |= std::string(reinterpret_cast<char*>(name)) == "id";
        found_label |= std::string(reinterpret_cast<char*>(name)) == "label";
    }
    EXPECT_TRUE(found_id);
    EXPECT_TRUE(found_label);
}

TEST_F(OdbcConformance, Diagnostics)
{
    ASSERT_EQ(SQL_ERROR, SQLExecDirect(stmt.value,
              sql_text("SELECT * FROM #odbc_conf_missing_table"), SQL_NTS));
    SQLCHAR state[SQL_SQLSTATE_SIZE + 1]{};
    SQLCHAR message[1024]{};
    SQLINTEGER native_error = 0;
    SQLSMALLINT length = 0;
    ASSERT_ODBC(SQLGetDiagRec(SQL_HANDLE_STMT, stmt.value, 1, state, &native_error,
                             message, sizeof(message), &length), SQL_HANDLE_STMT, stmt.value);
    EXPECT_EQ("42", std::string(reinterpret_cast<char*>(state), 2));
    EXPECT_GT(length, 0);
}

TEST_F(OdbcConformance, Unicode)
{
    static_assert(sizeof(SQLWCHAR) == 2, "unixODBC SQLWCHAR must be two bytes");
    SQLWCHAR value[] = {'G', 'r', 0x00fc, 0x00df, 'e', ' ', 0x4e16, 0x754c, 0};
    SQLWCHAR result[64]{};
    SQLLEN input_indicator = SQL_NTS, output_indicator = 0;
    ASSERT_NO_FATAL_FAILURE(execute(
        "CREATE TABLE #odbc_unicode (value NVARCHAR(100) NOT NULL)"));
    ASSERT_NO_FATAL_FAILURE(close_cursor());
    ASSERT_ODBC(SQLPrepare(stmt.value,
                          sql_text("INSERT INTO #odbc_unicode (value) VALUES (?)"), SQL_NTS),
                SQL_HANDLE_STMT, stmt.value);
    ASSERT_ODBC(SQLBindParameter(stmt.value, 1, SQL_PARAM_INPUT, SQL_C_WCHAR,
                                SQL_WVARCHAR, 100, 0, value, sizeof(value), &input_indicator),
                SQL_HANDLE_STMT, stmt.value);
    ASSERT_ODBC(SQLExecute(stmt.value), SQL_HANDLE_STMT, stmt.value);
    ASSERT_NO_FATAL_FAILURE(close_cursor());
    ASSERT_NO_FATAL_FAILURE(execute("SELECT value FROM #odbc_unicode"));
    ASSERT_ODBC(SQLFetch(stmt.value), SQL_HANDLE_STMT, stmt.value);
    ASSERT_ODBC(SQLGetData(stmt.value, 1, SQL_C_WCHAR, result, sizeof(result),
                          &output_indicator), SQL_HANDLE_STMT, stmt.value);
    ASSERT_EQ(static_cast<SQLLEN>(sizeof(value) - sizeof(SQLWCHAR)), output_indicator);
    EXPECT_TRUE(std::equal(std::begin(value), std::end(value), std::begin(result)));
    EXPECT_EQ(SQL_NO_DATA, SQLFetch(stmt.value));
}

struct alignas(std::max_align_t) DataBuffer {
    std::array<unsigned char, 40000> data{};
};

template<class T>
T read_value(const unsigned char* data)
{
    T value{};
    std::memcpy(&value, data, sizeof(value));
    return value;
}

class DataTypes : public OdbcConformance {
protected:
    std::vector<odbc_samples::Sample> samples = odbc_samples::all();
    std::array<odbc_samples::Bytes, 3> versions;

    void SetUp() override
    {
        ASSERT_NO_FATAL_FAILURE(OdbcConformance::SetUp());
        SQLCHAR version[64]{};
        ASSERT_ODBC(SQLGetInfo(dbc.value, SQL_DBMS_VER, version, sizeof(version), nullptr),
                    SQL_HANDLE_DBC, dbc.value);
        ASSERT_GE(std::stoi(reinterpret_cast<char*>(version)), 17)
            << "The complete datatype table requires SQL Server 2025 (including JSON and VECTOR)";

        std::string ddl = "CREATE TABLE #odbc_datatypes (id int NOT NULL";
        std::string names = "id";
        std::string values;
        std::string nulls;
        for (const auto& sample : samples) {
            ddl += ", [" + sample.name + "] " + sample.declaration +
                   (sample.generated ? " NOT NULL" : " NULL");
            if (!sample.generated) {
                names += ", [" + sample.name + "]";
                values += ", " + sample.expression;
                nulls += ", NULL";
            }
        }
        ddl += ")";
        ASSERT_NO_FATAL_FAILURE(execute(ddl.c_str()));
        ASSERT_NO_FATAL_FAILURE(close_cursor());
        for (int row = 1; row <= 3; ++row) {
            const auto insert = "INSERT INTO #odbc_datatypes (" + names + ") SELECT " +
                                std::to_string(row) + (row == 2 ? nulls : values);
            ASSERT_NO_FATAL_FAILURE(execute(insert.c_str()));
            ASSERT_NO_FATAL_FAILURE(close_cursor());
        }

        // Generated values have no literal oracle; snapshot their binary form once.
        ASSERT_NO_FATAL_FAILURE(execute(
            "SELECT CONVERT(binary(8),[rowversion]) FROM #odbc_datatypes ORDER BY id"));
        for (auto& value : versions) {
            ASSERT_ODBC(SQLFetch(stmt.value), SQL_HANDLE_STMT, stmt.value);
            value.resize(8);
            SQLLEN size = 0;
            ASSERT_ODBC(SQLGetData(stmt.value, 1, SQL_C_BINARY, value.data(), 8, &size),
                        SQL_HANDLE_STMT, stmt.value);
            ASSERT_EQ(8, size);
        }
        EXPECT_EQ(SQL_NO_DATA, SQLFetch(stmt.value));
        EXPECT_NE(versions[0], versions[1]);
        EXPECT_NE(versions[1], versions[2]);
        ASSERT_NO_FATAL_FAILURE(close_cursor());
    }

    void select(const odbc_samples::Sample& sample)
    {
        const auto sql = "SELECT [" + sample.name + "] FROM #odbc_datatypes ORDER BY id";
        ASSERT_NO_FATAL_FAILURE(execute(sql.c_str()));
    }

    void reset_statement()
    {
        ASSERT_NO_FATAL_FAILURE(close_cursor());
        ASSERT_ODBC(SQLFreeStmt(stmt.value, SQL_UNBIND), SQL_HANDLE_STMT, stmt.value);
    }

    void numeric_descriptor(const odbc_samples::Sample& sample, SQLPOINTER buffer)
    {
        if (sample.scale < 0) {
            return;
        }
        SQLHDESC descriptor = SQL_NULL_HDESC;
        ASSERT_ODBC(SQLGetStmtAttr(stmt.value, SQL_ATTR_APP_ROW_DESC, &descriptor, 0, nullptr),
                    SQL_HANDLE_STMT, stmt.value);
        ASSERT_ODBC(SQLSetDescField(descriptor, 1, SQL_DESC_TYPE,
                                   reinterpret_cast<SQLPOINTER>(SQL_C_NUMERIC), 0),
                    SQL_HANDLE_DESC, descriptor);
        ASSERT_ODBC(SQLSetDescField(descriptor, 1, SQL_DESC_PRECISION,
                                   reinterpret_cast<SQLPOINTER>(38), 0),
                    SQL_HANDLE_DESC, descriptor);
        ASSERT_ODBC(SQLSetDescField(descriptor, 1, SQL_DESC_SCALE,
                                   reinterpret_cast<SQLPOINTER>(static_cast<intptr_t>(sample.scale)), 0),
                    SQL_HANDLE_DESC, descriptor);
        if (buffer != nullptr) {
            ASSERT_ODBC(SQLSetDescField(descriptor, 1, SQL_DESC_DATA_PTR, buffer, 0),
                        SQL_HANDLE_DESC, descriptor);
        }
    }

    void expect_text(const odbc_samples::Sample& sample, const std::string& actual, size_t row)
    {
        if (sample.name == "vector") {
            // The default ODBC representation is a JSON array; exponent formatting can vary.
            std::istringstream input(actual);
            char open = 0, comma1 = 0, comma2 = 0, close = 0;
            double x = 0, y = 0, z = 0;
            ASSERT_TRUE(input >> open >> x >> comma1 >> y >> comma2 >> z >> close);
            EXPECT_EQ('[', open);
            EXPECT_EQ(',', comma1);
            EXPECT_EQ(',', comma2);
            EXPECT_EQ(']', close);
            EXPECT_EQ(1.0, x);
            EXPECT_EQ(2.0, y);
            EXPECT_EQ(3.0, z);
            input >> std::ws;
            EXPECT_TRUE(input.eof());
        } else if (sample.generated) {
            const char* digits = "0123456789ABCDEF";
            std::string expected;
            for (auto byte : versions[row]) {
                expected += digits[byte >> 4];
                expected += digits[byte & 15];
            }
            EXPECT_EQ(expected, actual);
        } else {
            EXPECT_EQ(sample.text, actual);
        }
    }

    void expect_value(const odbc_samples::Sample& sample, SQLSMALLINT type,
                      const unsigned char* data, SQLLEN length, size_t row)
    {
        if (row == 1 && !sample.generated) {
            EXPECT_EQ(SQL_NULL_DATA, length);
            return;
        }
        ASSERT_GE(length, 0);
        ASSERT_LT(length, static_cast<SQLLEN>(DataBuffer{}.data.size()));
        if (type == SQL_C_CHAR) {
            EXPECT_EQ(0, data[length]);
            ASSERT_NO_FATAL_FAILURE(expect_text(
                sample, std::string(reinterpret_cast<const char*>(data), length), row));
            return;
        }
        if (type == SQL_C_WCHAR) {
            ASSERT_EQ(0, length % sizeof(SQLWCHAR));
            std::string text;
            for (SQLLEN i = 0; i < length; i += sizeof(SQLWCHAR)) {
                const auto character = read_value<SQLWCHAR>(data + i);
                ASSERT_LE(character, 127) << "These datatype fixtures use ASCII text";
                text += static_cast<char>(character);
            }
            EXPECT_EQ(0, read_value<SQLWCHAR>(data + length));
            ASSERT_NO_FATAL_FAILURE(expect_text(sample, text, row));
            return;
        }
        const auto& expected = sample.generated ? versions[row] : sample.native;
        ASSERT_EQ(expected.size(), static_cast<size_t>(length));
        if (type == SQL_C_NUMERIC) {
            const auto actual = read_value<SQL_NUMERIC_STRUCT>(data);
            const auto value = read_value<SQL_NUMERIC_STRUCT>(expected.data());
            EXPECT_EQ(value.precision, actual.precision);
            EXPECT_EQ(value.scale, actual.scale);
            EXPECT_EQ(value.sign, actual.sign);
            EXPECT_TRUE(std::equal(std::begin(value.val), std::end(value.val), std::begin(actual.val)));
        } else if (type == SQL_C_TYPE_TIMESTAMP) {
            const auto actual = read_value<SQL_TIMESTAMP_STRUCT>(data);
            const auto value = read_value<SQL_TIMESTAMP_STRUCT>(expected.data());
            EXPECT_EQ(std::tie(value.year, value.month, value.day, value.hour, value.minute,
                               value.second, value.fraction),
                      std::tie(actual.year, actual.month, actual.day, actual.hour, actual.minute,
                               actual.second, actual.fraction));
        } else if (sample.name == "time") {
            const auto actual = read_value<SQL_SS_TIME2_STRUCT>(data);
            const auto value = read_value<SQL_SS_TIME2_STRUCT>(expected.data());
            EXPECT_EQ(std::tie(value.hour, value.minute, value.second, value.fraction),
                      std::tie(actual.hour, actual.minute, actual.second, actual.fraction));
        } else if (sample.name == "datetimeoffset") {
            const auto actual = read_value<SQL_SS_TIMESTAMPOFFSET_STRUCT>(data);
            const auto value = read_value<SQL_SS_TIMESTAMPOFFSET_STRUCT>(expected.data());
            EXPECT_EQ(std::tie(value.year, value.month, value.day, value.hour, value.minute,
                               value.second, value.fraction, value.timezone_hour, value.timezone_minute),
                      std::tie(actual.year, actual.month, actual.day, actual.hour, actual.minute,
                               actual.second, actual.fraction, actual.timezone_hour, actual.timezone_minute));
        } else {
            EXPECT_EQ(expected, odbc_samples::Bytes(data, data + length));
        }
    }

    void get_data(SQLSMALLINT text_type = 0)
    {
        for (const auto& sample : samples) {
            SCOPED_TRACE(sample.name);
            ASSERT_NO_FATAL_FAILURE(select(sample));
            for (size_t row = 0; row < 3; ++row) {
                SCOPED_TRACE(row);
                ASSERT_ODBC(SQLFetch(stmt.value), SQL_HANDLE_STMT, stmt.value);
                DataBuffer buffer;
                SQLLEN length = 0;
                const auto type = text_type == 0 ? sample.c_type : text_type;
                if (type == SQL_C_NUMERIC) {
                    ASSERT_NO_FATAL_FAILURE(numeric_descriptor(sample, nullptr));
                }
                const auto rc = SQLGetData(stmt.value, 1,
                    type == SQL_C_NUMERIC ? SQL_ARD_TYPE : type,
                    buffer.data.data(), buffer.data.size(), &length);
                ASSERT_ODBC(rc, SQL_HANDLE_STMT, stmt.value);
                ASSERT_EQ(SQL_SUCCESS, rc) << "Unexpected conversion or truncation warning";
                ASSERT_NO_FATAL_FAILURE(expect_value(sample, type, buffer.data.data(), length, row));
            }
            EXPECT_EQ(SQL_NO_DATA, SQLFetch(stmt.value));
            ASSERT_NO_FATAL_FAILURE(reset_statement());
        }
    }
};

TEST_F(DataTypes, GetDataNative)
{
    ASSERT_NO_FATAL_FAILURE(get_data());
}

TEST_F(DataTypes, GetDataChar)
{
    ASSERT_NO_FATAL_FAILURE(get_data(SQL_C_CHAR));
}

TEST_F(DataTypes, GetDataWideChar)
{
    ASSERT_NO_FATAL_FAILURE(get_data(SQL_C_WCHAR));
}

TEST_F(DataTypes, BindColumns)
{
    ASSERT_NO_FATAL_FAILURE(execute("SELECT * FROM #odbc_datatypes ORDER BY id"));
    SQLSMALLINT column_count = 0;
    ASSERT_ODBC(SQLNumResultCols(stmt.value, &column_count), SQL_HANDLE_STMT, stmt.value);
    ASSERT_EQ(samples.size() + 1, static_cast<size_t>(column_count));
    std::vector<DataBuffer> buffers(samples.size());
    std::vector<SQLLEN> lengths(samples.size());
    // Bind all columns together as wide text; native per-type binding is covered by rowsets.
    for (size_t i = 0; i < samples.size(); ++i) {
        ASSERT_ODBC(SQLBindCol(stmt.value, static_cast<SQLUSMALLINT>(i + 2), SQL_C_WCHAR,
                              buffers[i].data.data(), buffers[i].data.size(), &lengths[i]),
                    SQL_HANDLE_STMT, stmt.value);
    }
    for (size_t row = 0; row < 3; ++row) {
        const auto rc = SQLFetch(stmt.value);
        ASSERT_ODBC(rc, SQL_HANDLE_STMT, stmt.value);
        ASSERT_EQ(SQL_SUCCESS, rc);
        for (size_t i = 0; i < samples.size(); ++i) {
            SCOPED_TRACE(samples[i].name);
            SCOPED_TRACE(row);
            ASSERT_NO_FATAL_FAILURE(expect_value(
                samples[i], SQL_C_WCHAR, buffers[i].data.data(), lengths[i], row));
        }
    }
    EXPECT_EQ(SQL_NO_DATA, SQLFetch(stmt.value));
    ASSERT_NO_FATAL_FAILURE(reset_statement());
}

enum class RowBinding { ColumnWise, RowWise };

class DataTypeRowsets : public DataTypes, public testing::WithParamInterface<RowBinding> {};

TEST_P(DataTypeRowsets, FetchScroll)
{
    struct Cell {
        DataBuffer buffer;
        SQLLEN length = 0;
    };
    std::array<Cell, 2> rows;
    DataBuffer columns;
    std::array<SQLLEN, 2> lengths{};
    std::array<SQLUSMALLINT, 2> status{};
    SQLULEN fetched = 0;
    const bool row_wise = GetParam() == RowBinding::RowWise;
    ASSERT_ODBC(SQLSetStmtAttr(stmt.value, SQL_ATTR_ROW_ARRAY_SIZE,
                              reinterpret_cast<SQLPOINTER>(2), 0), SQL_HANDLE_STMT, stmt.value);
    ASSERT_ODBC(SQLSetStmtAttr(stmt.value, SQL_ATTR_ROW_BIND_TYPE,
                              reinterpret_cast<SQLPOINTER>(row_wise ? sizeof(Cell) : SQL_BIND_BY_COLUMN),
                              0), SQL_HANDLE_STMT, stmt.value);
    ASSERT_ODBC(SQLSetStmtAttr(stmt.value, SQL_ATTR_ROWS_FETCHED_PTR, &fetched, 0),
                SQL_HANDLE_STMT, stmt.value);
    ASSERT_ODBC(SQLSetStmtAttr(stmt.value, SQL_ATTR_ROW_STATUS_PTR, status.data(), 0),
                SQL_HANDLE_STMT, stmt.value);
    for (const auto& sample : samples) {
        SCOPED_TRACE(sample.name);
        ASSERT_NO_FATAL_FAILURE(select(sample));
        // Fixed C types use their structure size for column-wise array strides.
        const auto stride = sample.c_type == SQL_C_CHAR || sample.c_type == SQL_C_WCHAR ||
                            sample.c_type == SQL_C_BINARY
                            ? size_t{19000} : sample.native.size();
        auto* pointer = row_wise ? rows[0].buffer.data.data() : columns.data.data();
        auto* indicator = row_wise ? &rows[0].length : lengths.data();
        ASSERT_ODBC(SQLBindCol(stmt.value, 1, sample.c_type, pointer, stride, indicator),
                    SQL_HANDLE_STMT, stmt.value);
        ASSERT_NO_FATAL_FAILURE(numeric_descriptor(sample, pointer));
        for (size_t first = 0; first < 3; first += 2) {
            const auto rc = SQLFetchScroll(stmt.value, SQL_FETCH_NEXT, 0);
            ASSERT_ODBC(rc, SQL_HANDLE_STMT, stmt.value);
            ASSERT_EQ(SQL_SUCCESS, rc);
            ASSERT_EQ(first == 0 ? 2U : 1U, fetched);
            for (size_t i = 0; i < fetched; ++i) {
                SCOPED_TRACE(first + i);
                EXPECT_EQ(SQL_ROW_SUCCESS, status[i]);
                const auto* data = row_wise ? rows[i].buffer.data.data()
                                            : columns.data.data() + i * stride;
                const auto length = row_wise ? rows[i].length : lengths[i];
                ASSERT_NO_FATAL_FAILURE(expect_value(sample, sample.c_type, data, length, first + i));
            }
            if (fetched == 1) {
                EXPECT_EQ(SQL_ROW_NOROW, status[1]);
            }
        }
        EXPECT_EQ(SQL_NO_DATA, SQLFetchScroll(stmt.value, SQL_FETCH_NEXT, 0));
        EXPECT_EQ(0U, fetched);
        ASSERT_NO_FATAL_FAILURE(reset_statement());
    }
    ASSERT_ODBC(SQLSetStmtAttr(stmt.value, SQL_ATTR_ROWS_FETCHED_PTR, nullptr, 0),
                SQL_HANDLE_STMT, stmt.value);
    ASSERT_ODBC(SQLSetStmtAttr(stmt.value, SQL_ATTR_ROW_STATUS_PTR, nullptr, 0),
                SQL_HANDLE_STMT, stmt.value);
}

INSTANTIATE_TEST_SUITE_P(Retrieval, DataTypeRowsets,
    testing::Values(RowBinding::ColumnWise, RowBinding::RowWise),
    [](const testing::TestParamInfo<RowBinding>& info) {
        return info.param == RowBinding::ColumnWise ? "ColumnWise" : "RowWise";
    });

TEST_F(DataTypes, ChunkedGetData)
{
    for (const auto& sample : samples) {
        if (!sample.large) {
            continue;
        }
        SCOPED_TRACE(sample.name);
        ASSERT_NO_FATAL_FAILURE(select(sample));
        for (size_t row = 0; row < 3; ++row) {
            ASSERT_ODBC(SQLFetch(stmt.value), SQL_HANDLE_STMT, stmt.value);
            odbc_samples::Bytes result;
            const size_t terminator = sample.c_type == SQL_C_WCHAR ? sizeof(SQLWCHAR)
                                    : sample.c_type == SQL_C_CHAR ? 1 : 0;
            constexpr size_t chunk_size = 258;
            const size_t payload = chunk_size - terminator;
            bool finished = false;
            for (size_t chunk = 0; chunk < 100; ++chunk) {
                DataBuffer buffer;
                SQLLEN length = 0;
                const auto rc = SQLGetData(stmt.value, 1, sample.c_type,
                                          buffer.data.data(), chunk_size, &length);
                ASSERT_ODBC(rc, SQL_HANDLE_STMT, stmt.value);
                if (row == 1) {
                    EXPECT_EQ(SQL_NULL_DATA, length);
                    EXPECT_EQ(SQL_SUCCESS, rc);
                    finished = true;
                    break;
                }
                ASSERT_NE(SQL_NULL_DATA, length);
                size_t count = 0;
                if (rc == SQL_SUCCESS_WITH_INFO) {
                    SQLCHAR state[6]{};
                    SQLINTEGER native = 0;
                    SQLCHAR message[512]{};
                    ASSERT_ODBC(SQLGetDiagRec(SQL_HANDLE_STMT, stmt.value, 1, state, &native,
                                             message, sizeof(message), nullptr),
                                SQL_HANDLE_STMT, stmt.value);
                    EXPECT_STREQ("01004", reinterpret_cast<char*>(state));
                    EXPECT_TRUE(length == SQL_NO_TOTAL || length > static_cast<SQLLEN>(payload));
                    count = payload;
                } else {
                    ASSERT_GE(length, 0);
                    ASSERT_LE(length, static_cast<SQLLEN>(payload));
                    count = static_cast<size_t>(length);
                    finished = true;
                }
                result.insert(result.end(), buffer.data.begin(), buffer.data.begin() + count);
                if (terminator == 1) {
                    EXPECT_EQ(0, buffer.data[count]);
                } else if (terminator == sizeof(SQLWCHAR)) {
                    EXPECT_EQ(0, read_value<SQLWCHAR>(buffer.data.data() + count));
                }
                if (finished) {
                    break;
                }
            }
            ASSERT_TRUE(finished) << "Chunked retrieval did not terminate";
            if (row != 1) {
                EXPECT_EQ(sample.native, result);
            }
            DataBuffer buffer;
            SQLLEN length = 0;
            EXPECT_EQ(SQL_NO_DATA, SQLGetData(stmt.value, 1, sample.c_type,
                buffer.data.data(), chunk_size, &length));
        }
        EXPECT_EQ(SQL_NO_DATA, SQLFetch(stmt.value));
        ASSERT_NO_FATAL_FAILURE(reset_statement());
    }
}

TEST_F(OdbcConformance, TimestampAlias)
{
    // SQL Server permits only one rowversion/timestamp column per table.
    ASSERT_NO_FATAL_FAILURE(execute("CREATE TABLE #odbc_timestamp (id int, stamp timestamp)"));
    ASSERT_NO_FATAL_FAILURE(close_cursor());
    ASSERT_NO_FATAL_FAILURE(execute("INSERT INTO #odbc_timestamp(id) VALUES (1)"));
    ASSERT_NO_FATAL_FAILURE(close_cursor());
    ASSERT_NO_FATAL_FAILURE(execute("SELECT stamp FROM #odbc_timestamp"));
    ASSERT_ODBC(SQLFetch(stmt.value), SQL_HANDLE_STMT, stmt.value);
    std::array<unsigned char, 8> before{};
    SQLLEN length = 0;
    ASSERT_ODBC(SQLGetData(stmt.value, 1, SQL_C_BINARY, before.data(), before.size(), &length),
                SQL_HANDLE_STMT, stmt.value);
    ASSERT_EQ(8, length);
    ASSERT_NO_FATAL_FAILURE(close_cursor());
    ASSERT_NO_FATAL_FAILURE(execute("UPDATE #odbc_timestamp SET id=2"));
    ASSERT_NO_FATAL_FAILURE(close_cursor());
    ASSERT_NO_FATAL_FAILURE(execute("SELECT stamp FROM #odbc_timestamp"));
    std::array<unsigned char, 8> after{};
    ASSERT_ODBC(SQLBindCol(stmt.value, 1, SQL_C_BINARY, after.data(), after.size(), &length),
                SQL_HANDLE_STMT, stmt.value);
    ASSERT_ODBC(SQLFetch(stmt.value), SQL_HANDLE_STMT, stmt.value);
    ASSERT_EQ(8, length);
    EXPECT_NE(before, after);
    EXPECT_EQ(SQL_NO_DATA, SQLFetch(stmt.value));
}

} // namespace
