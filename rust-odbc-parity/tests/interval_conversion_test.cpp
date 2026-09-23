// Copyright (c) Microsoft Corporation. All rights reserved.

#include "odbc_test_fixture.h"

#include <array>
#include <cstddef>
#include <cstring>
#include <ostream>

namespace {

struct SourceType {
    const char* name;
    const char* declaration;
    SQLSMALLINT sql_type;
};

const SourceType kSources[] = {
    {"Tinyint", "tinyint", SQL_TINYINT},
    {"Smallint", "smallint", SQL_SMALLINT},
    {"Integer", "int", SQL_INTEGER},
    {"Bigint", "bigint", SQL_BIGINT},
    {"Decimal", "decimal(12,3)", SQL_DECIMAL},
    {"Numeric", "numeric(12,3)", SQL_NUMERIC},
};
const SourceType kApproximate[] = {
    {"Real", "real", SQL_REAL},
    {"Float", "float", SQL_DOUBLE},
};

struct TargetType {
    const char* name;
    SQLSMALLINT c_type;
    SQLINTERVAL interval_type;
};

const TargetType kTargets[] = {
    {"Year", SQL_C_INTERVAL_YEAR, SQL_IS_YEAR},
    {"Month", SQL_C_INTERVAL_MONTH, SQL_IS_MONTH},
    {"Day", SQL_C_INTERVAL_DAY, SQL_IS_DAY},
    {"Hour", SQL_C_INTERVAL_HOUR, SQL_IS_HOUR},
    {"Minute", SQL_C_INTERVAL_MINUTE, SQL_IS_MINUTE},
    {"Second", SQL_C_INTERVAL_SECOND, SQL_IS_SECOND},
};

struct IntervalCase {
    SourceType source;
    TargetType target;
    const char* scenario;
    const char* literal;
    SQLUINTEGER magnitude;
    bool negative;
    bool nullable_source;
    bool bound;
    SQLULEN version;
    SQLRETURN result;
    const char* state;
    SQLUINTEGER fraction = 0;
    std::array<SQLUINTEGER, 4> parts{};
    std::string sql_literal{};
};

void PrintTo(const IntervalCase& value, std::ostream* output) {
    *output << value.scenario << ", " << value.source.declaration << " -> "
            << value.target.name << ", " << (value.bound ? "BindCol" : "GetData");
}

std::vector<IntervalCase> Cases() {
    std::vector<IntervalCase> cases;
    for (const auto& source : kSources) {
        for (const auto& target : kTargets) {
            for (bool nullable : {false, true}) {
                for (bool bound : {false, true}) {
                    for (SQLULEN version : {SQLULEN{SQL_OV_ODBC3}, SQLULEN{SQL_OV_ODBC3_80}}) {
                        cases.push_back({source, target, "Exact", "12", 12, false,
                                         nullable, bound, version, SQL_SUCCESS, ""});
                    }
                }
            }
        }
    }
    for (const auto& target : kTargets) {
        for (bool bound : {false, true}) {
            cases.push_back({kSources[2], target, "Zero", "0", 0, false, true, bound,
                             SQL_OV_ODBC3_80, SQL_SUCCESS, ""});
            cases.push_back({kSources[2], target, "Negative", "-12", 12, true, true, bound,
                             SQL_OV_ODBC3_80, SQL_SUCCESS, ""});
            cases.push_back({kSources[2], target, "LeadingMax", "99", 99, false, true, bound,
                             SQL_OV_ODBC3_80, SQL_SUCCESS, ""});
            cases.push_back({kSources[2], target, "LeadingOverflow", "100", 100, false, true, bound,
                             SQL_OV_ODBC3_80, SQL_ERROR, "22015"});
            cases.push_back({kSources[2], target, "Null", nullptr, 0, false, true, bound,
                             SQL_OV_ODBC3_80, SQL_SUCCESS, ""});
            for (const auto& source : kApproximate) {
                cases.push_back({source, target, "ApproximateRejected", "12", 12, false,
                                 true, bound, SQL_OV_ODBC3_80, SQL_ERROR, "07006"});
            }
        }
    }
    return cases;
}

std::vector<IntervalCase> TextCases() {
    const SourceType sources[] = {
        {"Varchar", "varchar(128)", SQL_VARCHAR},
        {"Nvarchar", "nvarchar(128)", SQL_WVARCHAR},
    };
    const TargetType year_month{"YearToMonth", SQL_C_INTERVAL_YEAR_TO_MONTH, SQL_IS_YEAR_TO_MONTH};
    const TargetType day_hour{"DayToHour", SQL_C_INTERVAL_DAY_TO_HOUR, SQL_IS_DAY_TO_HOUR};
    const TargetType day_minute{"DayToMinute", SQL_C_INTERVAL_DAY_TO_MINUTE, SQL_IS_DAY_TO_MINUTE};
    const TargetType day_second{"DayToSecond", SQL_C_INTERVAL_DAY_TO_SECOND, SQL_IS_DAY_TO_SECOND};
    const TargetType hour_minute{"HourToMinute", SQL_C_INTERVAL_HOUR_TO_MINUTE, SQL_IS_HOUR_TO_MINUTE};
    const TargetType hour_second{"HourToSecond", SQL_C_INTERVAL_HOUR_TO_SECOND, SQL_IS_HOUR_TO_SECOND};
    const TargetType minute_second{"MinuteToSecond", SQL_C_INTERVAL_MINUTE_TO_SECOND, SQL_IS_MINUTE_TO_SECOND};
    struct Text {
        const char* name;
        const char* literal;
        TargetType target;
        SQLUINTEGER magnitude = 0;
        bool negative = false;
        SQLRETURN result = SQL_SUCCESS;
        const char* state = "";
        SQLUINTEGER fraction = 0;
        std::array<SQLUINTEGER, 4> parts{};
    };
    const Text texts[] = {
        {"SingleYear", "INTERVAL '12' YEAR", kTargets[0], 12},
        {"SingleMonth", "INTERVAL '12' MONTH", kTargets[1], 12},
        {"SingleDay", "INTERVAL '12' DAY", kTargets[2], 12},
        {"SingleHour", "INTERVAL '12' HOUR", kTargets[3], 12},
        {"SingleMinute", "INTERVAL '12' MINUTE", kTargets[4], 12},
        {"SingleSecond", "INTERVAL '12' SECOND", kTargets[5], 12},
        {"YearMonth", "INTERVAL '1-02' YEAR TO MONTH", year_month, 0, false, SQL_SUCCESS, "", 0, {1, 2}},
        {"DayHour", "INTERVAL '1 02' DAY TO HOUR", day_hour, 0, false, SQL_SUCCESS, "", 0, {1, 2}},
        {"DayMinute", "INTERVAL '1 02:03' DAY TO MINUTE", day_minute, 0, false, SQL_SUCCESS, "", 0, {1, 2, 3}},
        {"DaySecond", "INTERVAL '1 02:03:04.123' DAY TO SECOND", day_second, 0, false, SQL_SUCCESS, "", 123000000, {1, 2, 3, 4}},
        {"HourMinute", "INTERVAL '12:34' HOUR TO MINUTE", hour_minute, 0, false, SQL_SUCCESS, "", 0, {12, 34}},
        {"HourSecond", "INTERVAL '12:34:56.123' HOUR TO SECOND", hour_second, 0, false, SQL_SUCCESS, "", 123000000, {12, 34, 56}},
        {"MinuteSecond", "INTERVAL '12:34.123' MINUTE TO SECOND", minute_second, 0, false, SQL_SUCCESS, "", 123000000, {12, 34}},
        {"Negative", "INTERVAL -'12' DAY", kTargets[2], 12, true},
        {"LowerCase", "interval '12' day", kTargets[2], 12},
        {"Whitespace", "  INTERVAL '12' DAY  ", kTargets[2], 12},
        {"LeadingPrecisionOverflow", "INTERVAL '100' DAY(3)", kTargets[2], 100, false, SQL_ERROR, "22015"},
        {"FractionTruncated", "INTERVAL '1.1234567' SECOND(2,6)", kTargets[5], 1, false, SQL_SUCCESS_WITH_INFO, "01S07", 123456000},
        {"MonthOutOfRange", "INTERVAL '1-12' YEAR TO MONTH", year_month, 0, false, SQL_ERROR, "22018"},
        {"HourOutOfRange", "INTERVAL '1 24:00:00' DAY TO SECOND", day_second, 0, false, SQL_ERROR, "22018"},
        {"MinuteOutOfRange", "INTERVAL '1 00:60:00' DAY TO SECOND", day_second, 0, false, SQL_ERROR, "22018"},
        {"SecondOutOfRange", "INTERVAL '1 00:00:60' DAY TO SECOND", day_second, 0, false, SQL_ERROR, "22018"},
        {"TooManyFractionDigits", "INTERVAL '1.1234567890' SECOND(2,9)", kTargets[5], 0, false, SQL_ERROR, "22018"},
        {"MissingUnit", "INTERVAL '1'", kTargets[2], 0, false, SQL_ERROR, "22018"},
        {"MissingKeyword", "'1' DAY", kTargets[2], 0, false, SQL_ERROR, "22018"},
        {"TrailingJunk", "INTERVAL '1' DAY junk", kTargets[2], 0, false, SQL_ERROR, "22018"},
    };
    std::vector<IntervalCase> cases;
    for (const auto& text : texts) {
        std::string quoted = "'";
        for (const char* p = text.literal; *p; ++p) {
            quoted += *p;
            if (*p == '\'') {
                quoted += '\'';
            }
        }
        quoted += "'";
        for (const auto& source : sources) {
            for (bool bound : {false, true}) {
                for (SQLULEN version : {SQLULEN{SQL_OV_ODBC3}, SQLULEN{SQL_OV_ODBC3_80}}) {
                    IntervalCase value{source, text.target, text.name, text.literal,
                                       text.magnitude, text.negative, false, bound, version,
                                       text.result, text.state};
                    value.fraction = text.fraction;
                    value.parts = text.parts;
                    value.sql_literal = quoted;
                    cases.push_back(value);
                }
            }
        }
    }
    return cases;
}

std::string CaseName(const ::testing::TestParamInfo<IntervalCase>& info) {
    const auto& p = info.param;
    return std::string(p.scenario) + "_" + p.source.name + "_" + p.target.name +
           (p.nullable_source ? "_Nullable" : "_Literal") +
           (p.bound ? "_BindCol" : "_GetData") +
           (p.version == SQL_OV_ODBC3 ? "_Odbc30" : "_Odbc38");
}

class IntervalConversionTest
    : public ODBCTest, public ::testing::WithParamInterface<IntervalCase> {
protected:
    static constexpr unsigned char kGuard = 0xA5;
    static constexpr std::size_t kOffset = 16;
    alignas(std::max_align_t) std::array<unsigned char, sizeof(SQL_INTERVAL_STRUCT) + 32> buffer_{};
    SQLLEN indicator_ = -999;

    void SetUp() override {
        ASSERT_NO_FATAL_FAILURE(ODBCTest::SetUp());
        ASSERT_TRUE(ODBCTestConfig::Instance().HasConnection());
        ASSERT_SQL_OK(SQLSetEnvAttr(env_, SQL_ATTR_ODBC_VERSION,
                                    reinterpret_cast<SQLPOINTER>(GetParam().version), 0),
                      SQL_HANDLE_ENV, env_);
        ASSERT_NO_FATAL_FAILURE(Connect());
        SQLCHAR version[64] = {};
        SQLSMALLINT length = 0;
        ASSERT_SQL_OK(SQLGetInfoA(dbc_, SQL_DRIVER_VER, version, sizeof(version), &length),
                      SQL_HANDLE_DBC, dbc_);
        RecordProperty("driver_version", reinterpret_cast<const char*>(version));
        const bool text = GetParam().source.sql_type == SQL_VARCHAR ||
                          GetParam().source.sql_type == SQL_WVARCHAR;
        RecordProperty("oracle", text ? "ODBC SQL-to-C Character: interval literals" :
                                      "ODBC SQL-to-C Numeric: exact numeric to single-field interval");
        RecordProperty("reference_family", text ? "text-to-interval" :
                                                "exact-numeric-to-single-field-interval");
        RecordProperty("source_declaration", GetParam().source.declaration);
        RecordProperty("target_c_type", GetParam().target.c_type);
        RecordProperty("input", GetParam().literal ? GetParam().literal : "<NULL>");
    }

    void RecordResult(const char* phase, SQLRETURN result) {
        RecordProperty(std::string(phase) + "_return", result);
        RecordProperty(std::string(phase) + "_sqlstate",
                       ODBCTestUtils::GetDiagState(SQL_HANDLE_STMT, stmt_));
        RecordProperty(std::string(phase) + "_diagnostics",
                       ODBCTestUtils::GetDiagMessage(SQL_HANDLE_STMT, stmt_));
    }

    void CheckGuardBytes() {
        for (std::size_t index = 0; index < buffer_.size(); ++index) {
            if (index < kOffset || index >= kOffset + sizeof(SQL_INTERVAL_STRUCT)) {
                EXPECT_EQ(kGuard, buffer_[index]) << "outside interval object, byte " << index;
            }
        }
    }

    void RunContract();
};

void IntervalConversionTest::RunContract() {
    const auto& p = GetParam();
    const std::string literal = !p.sql_literal.empty() ? p.sql_literal :
                                p.literal ? p.literal : "NULL";
    std::string query;
    if (p.nullable_source) {
        query = "SET NOCOUNT ON; DECLARE @values TABLE(v " +
                std::string(p.source.declaration) + " NULL); INSERT @values VALUES (" +
                literal + "); SELECT v FROM @values";
    } else {
        query = "SELECT CAST(" + literal + " AS " + p.source.declaration + ") AS v";
    }
    ASSERT_NO_FATAL_FAILURE(ExecDirect(query));
    SQLSMALLINT columns = 0;
    ASSERT_SQL_OK(SQLNumResultCols(stmt_, &columns), SQL_HANDLE_STMT, stmt_);
    ASSERT_EQ(1, columns);
    SQLSMALLINT type = 0, scale = 0, nullable = 0;
    SQLULEN size = 0;
    ASSERT_SQL_OK(SQLDescribeCol(stmt_, 1, nullptr, 0, nullptr, &type, &size, &scale, &nullable),
                  SQL_HANDLE_STMT, stmt_);
    RecordProperty("reported_sql_type", type);
    RecordProperty("reported_nullable", nullable);
    if (p.source.sql_type == SQL_NUMERIC || p.source.sql_type == SQL_DECIMAL) {
        ASSERT_TRUE(type == SQL_NUMERIC || type == SQL_DECIMAL);
    } else if (p.source.sql_type == SQL_DOUBLE) {
        ASSERT_TRUE(type == SQL_FLOAT || type == SQL_DOUBLE);
    } else {
        ASSERT_EQ(p.source.sql_type, type);
    }
    if (p.nullable_source) {
        EXPECT_EQ(SQL_NULLABLE, nullable);
    }
    buffer_.fill(kGuard);
    if (p.bound) {
        const auto bound = SQLBindCol(stmt_, 1, p.target.c_type, buffer_.data() + kOffset,
                                      sizeof(SQL_INTERVAL_STRUCT), &indicator_);
        RecordResult("bind", bound);
        ASSERT_EQ(SQL_SUCCESS, bound)
            << ODBCTestUtils::GetDiagMessage(SQL_HANDLE_STMT, stmt_);
    }
    auto result = SQLFetch(stmt_);
    if (!p.bound) {
        ASSERT_EQ(SQL_SUCCESS, result) << StmtDiagState();
        result = SQLGetData(stmt_, 1, p.target.c_type, buffer_.data() + kOffset,
                            sizeof(SQL_INTERVAL_STRUCT), &indicator_);
    }
    RecordResult("convert", result);
    EXPECT_EQ(p.result, result) << ODBCTestUtils::GetDiagMessage(SQL_HANDLE_STMT, stmt_);
    EXPECT_EQ(std::string(p.state), StmtDiagState());
    CheckGuardBytes();
    if (SQL_SUCCEEDED(result)) {
        RecordProperty("indicator", std::to_string(indicator_));
        if (!p.literal) {
            EXPECT_EQ(SQL_NULL_DATA, indicator_);
        } else {
            EXPECT_EQ(static_cast<SQLLEN>(sizeof(SQL_INTERVAL_STRUCT)), indicator_);
            SQL_INTERVAL_STRUCT value{};
            std::memcpy(&value, buffer_.data() + kOffset, sizeof(value));
            RecordProperty("interval_type", static_cast<int>(value.interval_type));
            RecordProperty("interval_sign", value.interval_sign);
            EXPECT_EQ(p.target.interval_type, value.interval_type);
            EXPECT_EQ(p.negative ? SQL_TRUE : SQL_FALSE, value.interval_sign);
            SQLUINTEGER magnitude = 0;
            switch (p.target.interval_type) {
                case SQL_IS_YEAR: magnitude = value.intval.year_month.year; break;
                case SQL_IS_MONTH: magnitude = value.intval.year_month.month; break;
                case SQL_IS_DAY: magnitude = value.intval.day_second.day; break;
                case SQL_IS_HOUR: magnitude = value.intval.day_second.hour; break;
                case SQL_IS_MINUTE: magnitude = value.intval.day_second.minute; break;
                case SQL_IS_SECOND:
                    magnitude = value.intval.day_second.second;
                    EXPECT_EQ(p.fraction, value.intval.day_second.fraction);
                    break;
                case SQL_IS_YEAR_TO_MONTH:
                    EXPECT_EQ(p.parts[0], value.intval.year_month.year);
                    EXPECT_EQ(p.parts[1], value.intval.year_month.month);
                    break;
                case SQL_IS_DAY_TO_HOUR:
                case SQL_IS_DAY_TO_MINUTE:
                case SQL_IS_DAY_TO_SECOND:
                    EXPECT_EQ(p.parts[0], value.intval.day_second.day);
                    EXPECT_EQ(p.parts[1], value.intval.day_second.hour);
                    if (p.target.interval_type != SQL_IS_DAY_TO_HOUR) {
                        EXPECT_EQ(p.parts[2], value.intval.day_second.minute);
                    }
                    if (p.target.interval_type == SQL_IS_DAY_TO_SECOND) {
                        EXPECT_EQ(p.parts[3], value.intval.day_second.second);
                        EXPECT_EQ(p.fraction, value.intval.day_second.fraction);
                    }
                    break;
                case SQL_IS_HOUR_TO_MINUTE:
                case SQL_IS_HOUR_TO_SECOND:
                    EXPECT_EQ(p.parts[0], value.intval.day_second.hour);
                    EXPECT_EQ(p.parts[1], value.intval.day_second.minute);
                    if (p.target.interval_type == SQL_IS_HOUR_TO_SECOND) {
                        EXPECT_EQ(p.parts[2], value.intval.day_second.second);
                        EXPECT_EQ(p.fraction, value.intval.day_second.fraction);
                    }
                    break;
                case SQL_IS_MINUTE_TO_SECOND:
                    EXPECT_EQ(p.parts[0], value.intval.day_second.minute);
                    EXPECT_EQ(p.parts[1], value.intval.day_second.second);
                    EXPECT_EQ(p.fraction, value.intval.day_second.fraction);
                    break;
                default: FAIL() << "Unexpected interval target";
            }
            RecordProperty("magnitude", std::to_string(magnitude));
            if (SQL_SUCCEEDED(p.result)) {
                EXPECT_EQ(p.magnitude, magnitude);
            }
        }
    }
    // A rejected bound conversion can fail before the cursor advances.
    if (SQL_SUCCEEDED(result)) {
        EXPECT_EQ(SQL_NO_DATA, SQLFetch(stmt_));
    }
    ASSERT_SQL_OK(SQLFreeStmt(stmt_, SQL_UNBIND), SQL_HANDLE_STMT, stmt_);
    ASSERT_SQL_OK(SQLCloseCursor(stmt_), SQL_HANDLE_STMT, stmt_);
    ASSERT_NO_FATAL_FAILURE(ExecDirect("SELECT CAST(1 AS int) AS recovery"));
    ASSERT_EQ(SQL_SUCCESS, SQLFetch(stmt_));
    SQLINTEGER recovered = 0;
    SQLLEN recovered_length = 0;
    ASSERT_EQ(SQL_SUCCESS, SQLGetData(stmt_, 1, SQL_C_SLONG, &recovered, sizeof(recovered),
                                     &recovered_length));
    EXPECT_EQ(1, recovered);
    EXPECT_EQ(static_cast<SQLLEN>(sizeof(recovered)), recovered_length);
    ASSERT_SQL_OK(SQLCloseCursor(stmt_), SQL_HANDLE_STMT, stmt_);
}

TEST_P(IntervalConversionTest, ExactNumericContract) {
    ASSERT_NO_FATAL_FAILURE(RunContract());
}

class TextIntervalConversionTest : public IntervalConversionTest {};

TEST_P(TextIntervalConversionTest, LiteralContract) {
    ASSERT_NO_FATAL_FAILURE(RunContract());
}

INSTANTIATE_TEST_SUITE_P(Shared, IntervalConversionTest, ::testing::ValuesIn(Cases()), CaseName);
INSTANTIATE_TEST_SUITE_P(Shared, TextIntervalConversionTest, ::testing::ValuesIn(TextCases()), CaseName);

}  // namespace
