#include <sql.h>
#include <sqlext.h>

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

typedef struct {
    SQLHENV env;
    SQLHDBC dbc;
} connection;

typedef int (*test_fn)(connection *);

typedef struct {
    const char *name;
    test_fn run;
} test_case;

static void print_diagnostics(SQLSMALLINT handle_type, SQLHANDLE handle)
{
    SQLCHAR state[SQL_SQLSTATE_SIZE + 1];
    SQLCHAR message[1024];
    SQLINTEGER native_error;
    SQLSMALLINT message_length;
    SQLSMALLINT record = 1;
    SQLRETURN rc;

    while ((rc = SQLGetDiagRec(handle_type, handle, record, state,
                               &native_error, message, sizeof(message),
                               &message_length)) != SQL_NO_DATA) {
        if (!SQL_SUCCEEDED(rc)) {
            break;
        }
        fprintf(stderr, "  [%s] (%ld) %s\n", (char *)state,
                (long)native_error, (char *)message);
        ++record;
    }
}

static int call_succeeded(SQLRETURN rc, SQLSMALLINT handle_type,
                          SQLHANDLE handle, const char *expression,
                          const char *file, int line)
{
    if (SQL_SUCCEEDED(rc)) {
        return 1;
    }

    fprintf(stderr, "%s:%d: %s failed (SQLRETURN %d)\n",
            file, line, expression, (int)rc);
    if (handle != SQL_NULL_HANDLE) {
        print_diagnostics(handle_type, handle);
    }
    return 0;
}

#define ODBC_OK(expression, type, handle) \
    call_succeeded((expression), (type), (handle), #expression, __FILE__, __LINE__)

#define ASSERT_TRUE(condition, message)                                      \
    do {                                                                     \
        if (!(condition)) {                                                  \
            fprintf(stderr, "%s:%d: assertion failed: %s\n",                 \
                    __FILE__, __LINE__, (message));                           \
            return 0;                                                        \
        }                                                                    \
    } while (0)

static const char *env_or_default(const char *name, const char *default_value)
{
    const char *value = getenv(name);
    return value != NULL && value[0] != '\0' ? value : default_value;
}

static int open_connection(connection *conn)
{
    char generated[2048];
    const char *connection_string = getenv("ODBC_CONNECTION_STRING");
    SQLCHAR output[2048];
    SQLSMALLINT output_length;
    SQLRETURN rc;

    conn->env = SQL_NULL_HENV;
    conn->dbc = SQL_NULL_HDBC;

    rc = SQLAllocHandle(SQL_HANDLE_ENV, SQL_NULL_HANDLE, &conn->env);
    if (!ODBC_OK(rc, SQL_HANDLE_ENV, conn->env)) {
        return 0;
    }
    if (!ODBC_OK(SQLSetEnvAttr(conn->env, SQL_ATTR_ODBC_VERSION,
                               (SQLPOINTER)SQL_OV_ODBC3, 0),
                 SQL_HANDLE_ENV, conn->env)) {
        return 0;
    }
    if (!ODBC_OK(SQLAllocHandle(SQL_HANDLE_DBC, conn->env, &conn->dbc),
                 SQL_HANDLE_ENV, conn->env)) {
        return 0;
    }
    SQLSetConnectAttr(conn->dbc, SQL_LOGIN_TIMEOUT, (SQLPOINTER)10, 0);

    if (connection_string == NULL || connection_string[0] == '\0') {
        int length = snprintf(
            generated, sizeof(generated),
            "DRIVER={%s};SERVER=%s;UID=%s;PWD=%s;DATABASE=%s;"
            "Encrypt=yes;TrustServerCertificate=yes;",
            env_or_default("ODBC_DRIVER", "ODBC Driver 18 for SQL Server"),
            env_or_default("ODBC_SERVER", "127.0.0.1,1433"),
            env_or_default("ODBC_USER", "sa"),
            env_or_default("ODBC_PASSWORD", ""),
            env_or_default("ODBC_DATABASE", "tempdb"));
        if (length < 0 || (size_t)length >= sizeof(generated)) {
            fprintf(stderr, "ODBC connection settings are too long\n");
            return 0;
        }
        connection_string = generated;
    }

    rc = SQLDriverConnect(conn->dbc, NULL, (SQLCHAR *)connection_string,
                          SQL_NTS, output, sizeof(output), &output_length,
                          SQL_DRIVER_NOPROMPT);
    return ODBC_OK(rc, SQL_HANDLE_DBC, conn->dbc);
}

static void close_connection(connection *conn)
{
    if (conn->dbc != SQL_NULL_HDBC) {
        SQLDisconnect(conn->dbc);
        SQLFreeHandle(SQL_HANDLE_DBC, conn->dbc);
    }
    if (conn->env != SQL_NULL_HENV) {
        SQLFreeHandle(SQL_HANDLE_ENV, conn->env);
    }
}

static int allocate_statement(connection *conn, SQLHSTMT *stmt)
{
    *stmt = SQL_NULL_HSTMT;
    return ODBC_OK(SQLAllocHandle(SQL_HANDLE_STMT, conn->dbc, stmt),
                   SQL_HANDLE_DBC, conn->dbc);
}

static int execute(SQLHSTMT stmt, const char *sql)
{
    return ODBC_OK(SQLExecDirect(stmt, (SQLCHAR *)sql, SQL_NTS),
                   SQL_HANDLE_STMT, stmt);
}

static int scalar_integer(SQLHSTMT stmt, const char *sql, SQLINTEGER *value)
{
    SQLLEN indicator;

    if (!execute(stmt, sql)) {
        return 0;
    }
    if (!ODBC_OK(SQLFetch(stmt), SQL_HANDLE_STMT, stmt)) {
        return 0;
    }
    if (!ODBC_OK(SQLGetData(stmt, 1, SQL_C_SLONG, value, sizeof(*value),
                            &indicator),
                 SQL_HANDLE_STMT, stmt)) {
        return 0;
    }
    return indicator != SQL_NULL_DATA;
}

static int test_connection(connection *conn)
{
    SQLCHAR driver_name[256];
    SQLCHAR driver_version[256];
    SQLCHAR manager_version[256];
    SQLSMALLINT length;

    ASSERT_TRUE(ODBC_OK(SQLGetInfo(conn->dbc, SQL_DRIVER_NAME, driver_name,
                                   sizeof(driver_name), &length),
                        SQL_HANDLE_DBC, conn->dbc),
                "SQL_DRIVER_NAME must be available");
    ASSERT_TRUE(strstr((char *)driver_name, "msodbcsql") != NULL,
                "the active driver must be Microsoft msodbcsql");
    ASSERT_TRUE(ODBC_OK(SQLGetInfo(conn->dbc, SQL_DRIVER_VER, driver_version,
                                   sizeof(driver_version), &length),
                        SQL_HANDLE_DBC, conn->dbc),
                "SQL_DRIVER_VER must be available");
    ASSERT_TRUE(driver_version[0] != '\0', "driver version must not be empty");
    ASSERT_TRUE(ODBC_OK(SQLGetInfo(conn->dbc, SQL_DM_VER, manager_version,
                                   sizeof(manager_version), &length),
                        SQL_HANDLE_DBC, conn->dbc),
                "SQL_DM_VER must be available");
    ASSERT_TRUE(manager_version[0] != '\0',
                "driver manager version must not be empty");

    printf("driver=%s driver_version=%s driver_manager_version=%s\n",
           driver_name, driver_version, manager_version);
    return 1;
}

static int test_statements(connection *conn)
{
    SQLHSTMT stmt;
    SQLSMALLINT column_count;
    SQLCHAR column_name[64];
    SQLSMALLINT name_length;
    SQLSMALLINT data_type;
    SQLULEN column_size;
    SQLSMALLINT decimal_digits;
    SQLSMALLINT nullable;
    SQLINTEGER answer;
    SQLLEN indicator;

    ASSERT_TRUE(allocate_statement(conn, &stmt), "allocate statement");
    if (!execute(stmt, "SELECT CAST(42 AS INT) AS answer")) {
        SQLFreeHandle(SQL_HANDLE_STMT, stmt);
        return 0;
    }
    ASSERT_TRUE(ODBC_OK(SQLNumResultCols(stmt, &column_count),
                        SQL_HANDLE_STMT, stmt),
                "read result column count");
    ASSERT_TRUE(column_count == 1, "query must return one column");
    ASSERT_TRUE(ODBC_OK(SQLDescribeCol(stmt, 1, column_name,
                                      sizeof(column_name), &name_length,
                                      &data_type, &column_size,
                                      &decimal_digits, &nullable),
                        SQL_HANDLE_STMT, stmt),
                "describe result column");
    ASSERT_TRUE(strcmp((char *)column_name, "answer") == 0,
                "result alias must be preserved");
    ASSERT_TRUE(data_type == SQL_INTEGER, "result type must be SQL_INTEGER");
    ASSERT_TRUE(ODBC_OK(SQLFetch(stmt), SQL_HANDLE_STMT, stmt),
                "fetch result row");
    ASSERT_TRUE(ODBC_OK(SQLGetData(stmt, 1, SQL_C_SLONG, &answer,
                                  sizeof(answer), &indicator),
                        SQL_HANDLE_STMT, stmt),
                "read integer result");
    ASSERT_TRUE(answer == 42 && indicator != SQL_NULL_DATA,
                "statement result must equal 42");
    ASSERT_TRUE(SQLFetch(stmt) == SQL_NO_DATA, "query must return one row");
    SQLFreeHandle(SQL_HANDLE_STMT, stmt);
    return 1;
}

static int test_parameters(connection *conn)
{
    SQLHSTMT stmt;
    SQLINTEGER left = 20;
    SQLINTEGER right = 22;
    SQLINTEGER result;
    SQLLEN left_indicator = 0;
    SQLLEN right_indicator = 0;
    SQLLEN result_indicator;

    ASSERT_TRUE(allocate_statement(conn, &stmt), "allocate statement");
    ASSERT_TRUE(ODBC_OK(SQLPrepare(stmt,
                                   (SQLCHAR *)"SELECT CAST(? + ? AS INT)",
                                   SQL_NTS),
                        SQL_HANDLE_STMT, stmt),
                "prepare parameterized statement");
    ASSERT_TRUE(ODBC_OK(SQLBindParameter(stmt, 1, SQL_PARAM_INPUT, SQL_C_SLONG,
                                         SQL_INTEGER, 0, 0, &left, 0,
                                         &left_indicator),
                        SQL_HANDLE_STMT, stmt),
                "bind first parameter");
    ASSERT_TRUE(ODBC_OK(SQLBindParameter(stmt, 2, SQL_PARAM_INPUT, SQL_C_SLONG,
                                         SQL_INTEGER, 0, 0, &right, 0,
                                         &right_indicator),
                        SQL_HANDLE_STMT, stmt),
                "bind second parameter");
    ASSERT_TRUE(ODBC_OK(SQLExecute(stmt), SQL_HANDLE_STMT, stmt),
                "execute prepared statement");
    ASSERT_TRUE(ODBC_OK(SQLFetch(stmt), SQL_HANDLE_STMT, stmt),
                "fetch prepared result");
    ASSERT_TRUE(ODBC_OK(SQLGetData(stmt, 1, SQL_C_SLONG, &result,
                                  sizeof(result), &result_indicator),
                        SQL_HANDLE_STMT, stmt),
                "read prepared result");
    ASSERT_TRUE(result == 42, "bound parameters must produce 42");
    SQLFreeHandle(SQL_HANDLE_STMT, stmt);
    return 1;
}

static int test_transactions(connection *conn)
{
    SQLHSTMT stmt;
    SQLINTEGER count;
    int ok = 0;

    ASSERT_TRUE(allocate_statement(conn, &stmt), "allocate statement");
    execute(stmt, "DROP TABLE IF EXISTS dbo.odbc_conf_transactions");
    if (!execute(stmt, "CREATE TABLE dbo.odbc_conf_transactions "
                       "(value INT NOT NULL)")) {
        goto cleanup;
    }
    if (!ODBC_OK(SQLSetConnectAttr(conn->dbc, SQL_ATTR_AUTOCOMMIT,
                                   (SQLPOINTER)SQL_AUTOCOMMIT_OFF, 0),
                 SQL_HANDLE_DBC, conn->dbc)) {
        goto cleanup;
    }
    if (!execute(stmt, "INSERT INTO dbo.odbc_conf_transactions VALUES (1)")) {
        goto cleanup;
    }
    if (!ODBC_OK(SQLEndTran(SQL_HANDLE_DBC, conn->dbc, SQL_ROLLBACK),
                 SQL_HANDLE_DBC, conn->dbc)) {
        goto cleanup;
    }
    SQLCloseCursor(stmt);
    if (!scalar_integer(stmt,
                        "SELECT COUNT(*) FROM dbo.odbc_conf_transactions",
                        &count) || count != 0) {
        fprintf(stderr, "rollback did not remove the inserted row\n");
        goto cleanup;
    }
    SQLCloseCursor(stmt);
    if (!execute(stmt, "INSERT INTO dbo.odbc_conf_transactions VALUES (2)")) {
        goto cleanup;
    }
    if (!ODBC_OK(SQLEndTran(SQL_HANDLE_DBC, conn->dbc, SQL_COMMIT),
                 SQL_HANDLE_DBC, conn->dbc)) {
        goto cleanup;
    }
    SQLCloseCursor(stmt);
    if (!scalar_integer(stmt,
                        "SELECT COUNT(*) FROM dbo.odbc_conf_transactions",
                        &count) || count != 1) {
        fprintf(stderr, "commit did not retain the inserted row\n");
        goto cleanup;
    }
    ok = 1;

cleanup:
    SQLCloseCursor(stmt);
    SQLEndTran(SQL_HANDLE_DBC, conn->dbc, ok ? SQL_COMMIT : SQL_ROLLBACK);
    SQLSetConnectAttr(conn->dbc, SQL_ATTR_AUTOCOMMIT,
                      (SQLPOINTER)SQL_AUTOCOMMIT_ON, 0);
    execute(stmt, "DROP TABLE IF EXISTS dbo.odbc_conf_transactions");
    SQLFreeHandle(SQL_HANDLE_STMT, stmt);
    return ok;
}

static int test_metadata(connection *conn)
{
    SQLHSTMT stmt;
    SQLCHAR name[128];
    SQLLEN indicator;
    int found_table = 0;
    int found_id = 0;
    int found_label = 0;
    int ok = 0;

    ASSERT_TRUE(allocate_statement(conn, &stmt), "allocate statement");
    execute(stmt, "DROP TABLE IF EXISTS dbo.odbc_conf_metadata");
    if (!execute(stmt, "CREATE TABLE dbo.odbc_conf_metadata "
                       "(id INT NOT NULL, label NVARCHAR(50) NULL)")) {
        goto cleanup;
    }
    SQLCloseCursor(stmt);
    if (!ODBC_OK(SQLTables(stmt, NULL, 0, (SQLCHAR *)"dbo", SQL_NTS,
                           (SQLCHAR *)"odbc_conf_metadata", SQL_NTS,
                           (SQLCHAR *)"TABLE", SQL_NTS),
                 SQL_HANDLE_STMT, stmt)) {
        goto cleanup;
    }
    while (SQLFetch(stmt) != SQL_NO_DATA) {
        if (!ODBC_OK(SQLGetData(stmt, 3, SQL_C_CHAR, name, sizeof(name),
                                &indicator),
                     SQL_HANDLE_STMT, stmt)) {
            goto cleanup;
        }
        if (strcmp((char *)name, "odbc_conf_metadata") == 0) {
            found_table = 1;
        }
    }
    SQLCloseCursor(stmt);
    if (!ODBC_OK(SQLColumns(stmt, NULL, 0, (SQLCHAR *)"dbo", SQL_NTS,
                            (SQLCHAR *)"odbc_conf_metadata", SQL_NTS,
                            NULL, 0),
                 SQL_HANDLE_STMT, stmt)) {
        goto cleanup;
    }
    while (SQLFetch(stmt) != SQL_NO_DATA) {
        if (!ODBC_OK(SQLGetData(stmt, 4, SQL_C_CHAR, name, sizeof(name),
                                &indicator),
                     SQL_HANDLE_STMT, stmt)) {
            goto cleanup;
        }
        found_id |= strcmp((char *)name, "id") == 0;
        found_label |= strcmp((char *)name, "label") == 0;
    }
    if (!found_table || !found_id || !found_label) {
        fprintf(stderr, "metadata did not contain the expected table/columns\n");
        goto cleanup;
    }
    ok = 1;

cleanup:
    SQLCloseCursor(stmt);
    execute(stmt, "DROP TABLE IF EXISTS dbo.odbc_conf_metadata");
    SQLFreeHandle(SQL_HANDLE_STMT, stmt);
    return ok;
}

static int test_diagnostics(connection *conn)
{
    SQLHSTMT stmt;
    SQLRETURN rc;
    SQLCHAR state[SQL_SQLSTATE_SIZE + 1];
    SQLCHAR message[1024];
    SQLINTEGER native_error;
    SQLSMALLINT message_length;

    ASSERT_TRUE(allocate_statement(conn, &stmt), "allocate statement");
    rc = SQLExecDirect(stmt,
                       (SQLCHAR *)"SELECT * FROM dbo.odbc_conf_missing_table",
                       SQL_NTS);
    ASSERT_TRUE(rc == SQL_ERROR, "invalid query must return SQL_ERROR");
    ASSERT_TRUE(ODBC_OK(SQLGetDiagRec(SQL_HANDLE_STMT, stmt, 1, state,
                                      &native_error, message,
                                      sizeof(message), &message_length),
                        SQL_HANDLE_STMT, stmt),
                "retrieve diagnostic record");
    ASSERT_TRUE(state[0] == '4' && state[1] == '2',
                "missing table must report a 42xxx syntax/access SQLSTATE");
    ASSERT_TRUE(message_length > 0, "diagnostic message must not be empty");
    SQLFreeHandle(SQL_HANDLE_STMT, stmt);
    return 1;
}

static int sqlwchar_equal(const SQLWCHAR *left, const SQLWCHAR *right)
{
    while (*left != 0 && *right != 0) {
        if (*left != *right) {
            return 0;
        }
        ++left;
        ++right;
    }
    return *left == *right;
}

static int test_unicode(connection *conn)
{
    static SQLWCHAR value[] = {
        'G', 'r', 0x00fc, 0x00df, 'e', ' ', 0x4e16, 0x754c, 0
    };
    SQLWCHAR result[64];
    SQLHSTMT stmt;
    SQLLEN input_indicator = SQL_NTS;
    SQLLEN output_indicator;
    int ok = 0;

    ASSERT_TRUE(sizeof(SQLWCHAR) == 2,
                "unixODBC SQLWCHAR must use the two-byte ODBC representation");
    ASSERT_TRUE(allocate_statement(conn, &stmt), "allocate statement");
    execute(stmt, "DROP TABLE IF EXISTS dbo.odbc_conf_unicode");
    if (!execute(stmt, "CREATE TABLE dbo.odbc_conf_unicode "
                       "(value NVARCHAR(100) NOT NULL)")) {
        goto cleanup;
    }
    SQLCloseCursor(stmt);
    if (!ODBC_OK(SQLPrepare(stmt,
                            (SQLCHAR *)"INSERT INTO dbo.odbc_conf_unicode "
                                      "(value) VALUES (?)",
                            SQL_NTS),
                 SQL_HANDLE_STMT, stmt)) {
        goto cleanup;
    }
    if (!ODBC_OK(SQLBindParameter(stmt, 1, SQL_PARAM_INPUT, SQL_C_WCHAR,
                                  SQL_WVARCHAR, 100, 0, value, sizeof(value),
                                  &input_indicator),
                 SQL_HANDLE_STMT, stmt)) {
        goto cleanup;
    }
    if (!ODBC_OK(SQLExecute(stmt), SQL_HANDLE_STMT, stmt)) {
        goto cleanup;
    }
    SQLCloseCursor(stmt);
    if (!execute(stmt, "SELECT value FROM dbo.odbc_conf_unicode")) {
        goto cleanup;
    }
    if (!ODBC_OK(SQLFetch(stmt), SQL_HANDLE_STMT, stmt)) {
        goto cleanup;
    }
    memset(result, 0, sizeof(result));
    if (!ODBC_OK(SQLGetData(stmt, 1, SQL_C_WCHAR, result, sizeof(result),
                            &output_indicator),
                 SQL_HANDLE_STMT, stmt)) {
        goto cleanup;
    }
    if (!sqlwchar_equal(value, result)) {
        fprintf(stderr, "Unicode value changed during the ODBC round trip\n");
        goto cleanup;
    }
    ok = 1;

cleanup:
    SQLCloseCursor(stmt);
    execute(stmt, "DROP TABLE IF EXISTS dbo.odbc_conf_unicode");
    SQLFreeHandle(SQL_HANDLE_STMT, stmt);
    return ok;
}

static const test_case tests[] = {
    {"connection", test_connection},
    {"statements", test_statements},
    {"parameters", test_parameters},
    {"transactions", test_transactions},
    {"metadata", test_metadata},
    {"diagnostics", test_diagnostics},
    {"unicode", test_unicode},
};

static const test_case *find_test(const char *name)
{
    size_t i;
    for (i = 0; i < sizeof(tests) / sizeof(tests[0]); ++i) {
        if (strcmp(tests[i].name, name) == 0) {
            return &tests[i];
        }
    }
    return NULL;
}

int main(int argc, char **argv)
{
    const test_case *selected;
    connection conn;
    int passed;

    if (argc != 2) {
        fprintf(stderr, "usage: %s TEST\navailable tests:", argv[0]);
        for (size_t i = 0; i < sizeof(tests) / sizeof(tests[0]); ++i) {
            fprintf(stderr, " %s", tests[i].name);
        }
        fputc('\n', stderr);
        return 2;
    }
    selected = find_test(argv[1]);
    if (selected == NULL) {
        fprintf(stderr, "unknown test: %s\n", argv[1]);
        return 2;
    }
    if (!open_connection(&conn)) {
        close_connection(&conn);
        return 1;
    }
    passed = selected->run(&conn);
    close_connection(&conn);
    printf("%s: %s\n", selected->name, passed ? "PASS" : "FAIL");
    return passed ? 0 : 1;
}
