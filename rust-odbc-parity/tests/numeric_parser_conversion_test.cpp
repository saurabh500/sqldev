#include "odbc_test_fixture.h"

#include <algorithm>
#include <array>
#include <cstddef>
#include <cstring>
#include <iomanip>
#include <limits>
#include <locale>
#include <sstream>
#include <type_traits>
#include <variant>

namespace {

using ExpectedValue = std::variant<SQLBIGINT, SQLUBIGINT, SQLDOUBLE>;

enum class SourceWidth { Both, Narrow, Wide };

struct Scenario {
    const char* name;
    const char* literal;
    SQLSMALLINT target;
    SQLRETURN result;
    const char* state;
    ExpectedValue value;
    const char* decision;
    SourceWidth source = SourceWidth::Both;
    bool empty_delivery = false;
    const char* expression = nullptr;
    SQLULEN column_size = 128;
};

Scenario Exact(const char* name, const char* literal, SQLSMALLINT target,
               ExpectedValue value, const char* decision = "ConvertToFixed.range") {
    return {name, literal, target, SQL_SUCCESS, "", value, decision};
}

Scenario Fraction(const char* name, const char* literal, SQLSMALLINT target,
                  ExpectedValue value) {
    return {name, literal, target, SQL_SUCCESS_WITH_INFO, "01S07", value,
            "CharToBigint.fraction"};
}

Scenario Rejected(const char* name, const char* literal, SQLSMALLINT target,
                  const char* state, const char* decision = "ConvertToFixed.range") {
    return {name, literal, target, SQL_ERROR, state, SQLBIGINT{0}, decision};
}

Scenario SourceOnly(Scenario scenario, SourceWidth source) {
    scenario.source = source;
    return scenario;
}

std::vector<Scenario> Scenarios() {
    static const std::string buffer_at_capacity(622, '1');
    static const std::string buffer_above_capacity(623, '1');
    return {
        Exact("SignedByteMin", "-128", SQL_C_STINYINT, SQLBIGINT{-128}),
        Exact("SignedByteMax", "127", SQL_C_STINYINT, SQLBIGINT{127}),
        Rejected("SignedByteBelowMin", "-129", SQL_C_STINYINT, "22003"),
        Rejected("SignedByteAboveMax", "128", SQL_C_STINYINT, "22003"),
        Exact("UnsignedByteMax", "255", SQL_C_UTINYINT, SQLUBIGINT{255}),
        Rejected("UnsignedByteAboveMax", "256", SQL_C_UTINYINT, "22003"),
        Rejected("UnsignedByteNegative", "-1", SQL_C_UTINYINT, "22003"),
        Exact("ShortMin", "-32768", SQL_C_SSHORT, SQLBIGINT{-32768}),
        Exact("ShortMax", "32767", SQL_C_SSHORT, SQLBIGINT{32767}),
        Rejected("ShortBelowMin", "-32769", SQL_C_SSHORT, "22003"),
        Rejected("ShortAboveMax", "32768", SQL_C_SSHORT, "22003"),
        Exact("UnsignedShortMax", "65535", SQL_C_USHORT, SQLUBIGINT{65535}),
        Rejected("UnsignedShortAboveMax", "65536", SQL_C_USHORT, "22003"),
        Rejected("UnsignedShortNegative", "-1", SQL_C_USHORT, "22003"),
        Exact("LongMin", "-2147483648", SQL_C_SLONG, SQLBIGINT{-2147483647 - 1}),
        Exact("LongMax", "2147483647", SQL_C_SLONG, SQLBIGINT{2147483647}),
        Rejected("LongBelowMin", "-2147483649", SQL_C_SLONG, "22003"),
        Rejected("LongAboveMax", "2147483648", SQL_C_SLONG, "22003"),
        Exact("UnsignedLongMax", "4294967295", SQL_C_ULONG, SQLUBIGINT{4294967295ULL}),
        Rejected("UnsignedLongAboveMax", "4294967296", SQL_C_ULONG, "22003"),
        Rejected("UnsignedLongNegative", "-1", SQL_C_ULONG, "22003"),
        Exact("BigintMin", "-9223372036854775808", SQL_C_SBIGINT,
              (std::numeric_limits<SQLBIGINT>::min)(), "CharToBigint.signed_last_digit"),
        Exact("BigintMax", "9223372036854775807", SQL_C_SBIGINT,
              (std::numeric_limits<SQLBIGINT>::max)(), "CharToBigint.signed_last_digit"),
        Rejected("BigintBelowMin", "-9223372036854775809", SQL_C_SBIGINT, "22003",
                 "CharToBigint.signed_last_digit"),
        Rejected("BigintAboveMax", "9223372036854775808", SQL_C_SBIGINT, "22003"),
        Exact("UnsignedBigintMax", "18446744073709551615", SQL_C_UBIGINT,
              (std::numeric_limits<SQLUBIGINT>::max)(), "CharToBigint.unsigned_last_digit"),
        Rejected("UnsignedBigintAboveMax", "18446744073709551616", SQL_C_UBIGINT, "22003",
                 "CharToBigint.unsigned_last_digit"),
        Rejected("AccumulatorOverflowBeforeLastDigit", "999999999999999999999",
                 SQL_C_UBIGINT, "22003", "CharToBigint.accumulator"),
        Rejected("SignedAccumulatorOverflowBeforeLastDigit", "-999999999999999999999",
                 SQL_C_SBIGINT, "22003", "CharToBigint.signed_accumulator"),
        Rejected("UnsignedBigintNegative", "-1", SQL_C_UBIGINT, "22003"),
        Exact("NegativeZeroUnsigned", "-0", SQL_C_UBIGINT, SQLUBIGINT{0},
              "CharToBigint.sign"),
        Exact("LeadingPlus", "+42", SQL_C_SLONG, SQLBIGINT{42}, "CharToBigint.sign"),
        Exact("LeadingZerosAndSpaces", "  00042  ", SQL_C_SLONG, SQLBIGINT{42},
              "FindSigNumber.trim"),
        Exact("DiscardedZeroFraction", "12.000", SQL_C_SLONG, SQLBIGINT{12},
              "CharToBigint.fraction"),
        Fraction("DiscardedNonzeroFraction", "12.001", SQL_C_SLONG, SQLBIGINT{12}),
        Fraction("NegativeFraction", "-12.001", SQL_C_SLONG, SQLBIGINT{-12}),
        Fraction("FractionAtShortMax", "32767.9", SQL_C_SSHORT, SQLBIGINT{32767}),
        Fraction("FractionAtShortMin", "-32768.9", SQL_C_SSHORT, SQLBIGINT{-32768}),
        Fraction("NegativeFractionUnsigned", "-0.1", SQL_C_UTINYINT, SQLUBIGINT{0}),
        Exact("BitZero", "0", SQL_C_BIT, SQLUBIGINT{0}),
        Exact("BitOne", "1", SQL_C_BIT, SQLUBIGINT{1}),
        Rejected("BitAboveOne", "2", SQL_C_BIT, "22003"),
        Rejected("BitNegativeFraction", "-0.1", SQL_C_BIT, "22003",
                 "ConvertToFixed.negative_fraction_to_bit"),
        Fraction("BitPositiveFraction", "1.1", SQL_C_BIT, SQLUBIGINT{1}),
        Exact("ExponentInteger", "1e2", SQL_C_SLONG, SQLBIGINT{100},
              "ConvertToFixed.exponent"),
        Exact("ExponentUppercase", "1E+2", SQL_C_SLONG, SQLBIGINT{100},
              "ConvertToFixed.exponent"),
        Fraction("ExponentFraction", "1e-2", SQL_C_SLONG, SQLBIGINT{0}),
        Fraction("NegativeExponentFraction", "-1e-2", SQL_C_UTINYINT, SQLUBIGINT{0}),
        Rejected("ExponentOverflow", "1e309", SQL_C_DOUBLE, "22003",
                 "CharToDouble.overflow"),
        Rejected("ExponentUnderflow", "1e-999", SQL_C_DOUBLE, "22003",
                 "CharToDouble.underflow"),
        Rejected("IncompleteExponent", "1e", SQL_C_SLONG, "22018",
                 "CharToDouble.grammar"),
        Rejected("ExponentWithoutMantissa", "e2", SQL_C_SLONG, "22018",
                 "CharToDouble.grammar"),
        // Both native builds bypass numeric conversion for zero-length column data.
        {"EmptyLiteral", "", SQL_C_SLONG, SQL_SUCCESS, "", SQLBIGINT{0},
         "GetColData.empty", SourceWidth::Both, true},
        SourceOnly(Rejected("BlankOnly", " ", SQL_C_SLONG, "22018",
                            "CharToBigint.empty_after_trim"), SourceWidth::Narrow),
        SourceOnly(Rejected("BlankOnly", " ", SQL_C_SLONG, "HY000",
                            "ConvertToFixed.empty_wide_after_trim"), SourceWidth::Wide),
        {"EmbeddedNul", "12\\0x", SQL_C_SLONG, SQL_SUCCESS, "", SQLBIGINT{12},
         "CharToBigint.nul_termination", SourceWidth::Both, false,
         "'12' + CHAR(0) + 'x'"},
        Rejected("RepeatedSign", "--1", SQL_C_SLONG, "22018", "CharToBigint.grammar"),
        Rejected("RepeatedDecimalPoint", "1.2.3", SQL_C_SLONG, "22018",
                 "CharToBigint.grammar"),
        Rejected("TrailingJunk", "12x", SQL_C_SLONG, "22018", "CharToBigint.grammar"),
        Rejected("InteriorSpace", "1 2", SQL_C_SLONG, "22018", "CharToBigint.grammar"),
        Rejected("CommaSeparator", "1,234", SQL_C_SLONG, "22018", "CharToBigint.grammar"),
        Exact("DoubleDecimal", "12.5", SQL_C_DOUBLE, SQLDOUBLE{12.5}, "CharToDouble"),
        Exact("DoubleExponent", "-1.25e2", SQL_C_DOUBLE, SQLDOUBLE{-125},
              "CharToDouble"),
        SourceOnly(Exact("DoubleNegativeDotZero", "-.0", SQL_C_DOUBLE, SQLDOUBLE{0},
                         "CharToDouble.negative_dot_zero"), SourceWidth::Narrow),
        SourceOnly(Rejected("DoubleNegativeDotZero", "-.0", SQL_C_DOUBLE, "22018",
                            "ConvertToFloat.wide_negative_dot_zero"), SourceWidth::Wide),
        {"DoubleConversionBufferAtCapacity", buffer_at_capacity.c_str(), SQL_C_DOUBLE,
         SQL_ERROR, "22003", SQLBIGINT{0}, "CharToDouble.converted_capacity",
         SourceWidth::Narrow, false, nullptr, 1024},
        {"DoubleConversionBufferAboveCapacity", buffer_above_capacity.c_str(), SQL_C_DOUBLE,
         SQL_ERROR, "22003", SQLBIGINT{0}, "CharToDouble.input_capacity",
         SourceWidth::Narrow, false, nullptr, 1024},
        Exact("NullInteger", nullptr, SQL_C_SLONG, SQLBIGINT{0}, "GetColData.null"),
        Exact("NumericStructInteger", "42", SQL_C_NUMERIC, SQLUBIGINT{42},
              "ConvertToNumeric"),
    };
}

struct Parameters {
    Scenario scenario;
    bool wide;
    bool bound;
    SQLULEN version;
};

void PrintTo(const Parameters& p, std::ostream* output) {
    *output << p.scenario.name << ", " << (p.wide ? "nvarchar" : "varchar")
            << ", " << (p.bound ? "SQLBindCol" : "SQLGetData")
            << ", ODBC " << (p.version == SQL_OV_ODBC3 ? "3.0" : "3.8");
}

std::vector<Parameters> ParametersForAllPaths() {
    std::vector<Parameters> parameters;
    for (const auto& scenario : Scenarios()) {
        for (bool wide : {false, true}) {
            if ((wide && scenario.source == SourceWidth::Narrow) ||
                (!wide && scenario.source == SourceWidth::Wide)) {
                continue;
            }
            for (bool bound : {false, true}) {
                for (SQLULEN version : {SQLULEN{SQL_OV_ODBC3}, SQLULEN{SQL_OV_ODBC3_80}}) {
                    parameters.push_back({scenario, wide, bound, version});
                }
            }
        }
    }
    return parameters;
}

std::string ParameterName(const ::testing::TestParamInfo<Parameters>& info) {
    const auto& p = info.param;
    return std::string(p.scenario.name) + (p.wide ? "_Nvarchar" : "_Varchar") +
           (p.bound ? "_BindCol" : "_GetData") +
           (p.version == SQL_OV_ODBC3 ? "_Odbc30" : "_Odbc38");
}

std::string QuoteLiteral(const char* literal) {
    if (!literal) {
        return "NULL";
    }
    std::string result = "'";
    for (const char* p = literal; *p; ++p) {
        result += *p;
        if (*p == '\'') {
            result += '\'';
        }
    }
    return result + "'";
}

SQLLEN TargetSize(SQLSMALLINT target) {
    switch (target) {
        case SQL_C_STINYINT: return sizeof(signed char);
        case SQL_C_UTINYINT:
        case SQL_C_BIT: return sizeof(SQLCHAR);
        case SQL_C_SSHORT: return sizeof(SQLSMALLINT);
        case SQL_C_USHORT: return sizeof(SQLUSMALLINT);
        case SQL_C_SLONG: return sizeof(SQLINTEGER);
        case SQL_C_ULONG: return sizeof(SQLUINTEGER);
        case SQL_C_SBIGINT: return sizeof(SQLBIGINT);
        case SQL_C_UBIGINT: return sizeof(SQLUBIGINT);
        case SQL_C_DOUBLE: return sizeof(SQLDOUBLE);
        case SQL_C_NUMERIC: return sizeof(SQL_NUMERIC_STRUCT);
        default:
            ADD_FAILURE() << "Unhandled C target " << target;
            return 0;
    }
}

class NumericParserConversionTest
    : public ODBCTest, public ::testing::WithParamInterface<Parameters> {
protected:
    static constexpr unsigned char kGuard = 0xA5;
    static constexpr std::size_t kOffset = 16;
    alignas(std::max_align_t) std::array<unsigned char, 64> buffer_{};
    struct {
        SQLLEN before = 0x12345678;
        SQLLEN value = -999;
        SQLLEN after = 0x23456789;
    } indicator_;

    void SetUp() override {
        ASSERT_NO_FATAL_FAILURE(ODBCTest::SetUp());
        ASSERT_TRUE(ODBCTestConfig::Instance().HasConnection())
            << "Set ODBC_TEST_SERVER and connection credentials";
        ASSERT_SQL_OK(SQLSetEnvAttr(env_, SQL_ATTR_ODBC_VERSION,
                                    reinterpret_cast<SQLPOINTER>(GetParam().version), 0),
                      SQL_HANDLE_ENV, env_);
        ASSERT_NO_FATAL_FAILURE(Connect());
        ASSERT_NO_FATAL_FAILURE(RecordInfo(SQL_DRIVER_VER, "driver_version"));
        ASSERT_NO_FATAL_FAILURE(RecordInfo(SQL_DRIVER_NAME, "driver_name"));
        ASSERT_NO_FATAL_FAILURE(RecordInfo(SQL_DM_VER, "driver_manager_version"));
        ASSERT_NO_FATAL_FAILURE(RecordInfo(SQL_DBMS_VER, "server_version"));
        RecordProperty("reference_decision", GetParam().scenario.decision);
        RecordProperty("source_literal",
                       GetParam().scenario.literal ? GetParam().scenario.literal : "<NULL>");
        RecordProperty("target_c_type", GetParam().scenario.target);
        if (GetParam().scenario.expression) {
            RecordProperty("source_expression", GetParam().scenario.expression);
        }
    }

    void RecordInfo(SQLUSMALLINT key, const char* property) {
        SQLTCHAR value[128] = {};
        SQLSMALLINT length = 0;
        ASSERT_SQL_OK(SQLGetInfo(dbc_, key, value, sizeof(value), &length),
                      SQL_HANDLE_DBC, dbc_);
        RecordProperty(property, ODBCTestUtils::ToNarrow(SqlTString(value)));
    }

    void CheckGuards(SQLLEN width) {
        for (std::size_t i = 0; i < buffer_.size(); ++i) {
            if (i < kOffset || i >= kOffset + static_cast<std::size_t>(width)) {
                EXPECT_EQ(kGuard, buffer_[i]) << "write outside C target at byte " << i;
            }
        }
        EXPECT_EQ(SQLLEN{0x12345678}, indicator_.before);
        EXPECT_EQ(SQLLEN{0x23456789}, indicator_.after);
    }

    template <typename T>
    void CheckScalar(const ExpectedValue& expected, const char* prefix) {
        T actual{};
        std::memcpy(&actual, buffer_.data() + kOffset, sizeof(actual));
        std::ostringstream rendered;
        rendered.imbue(std::locale::classic());
        if constexpr (std::is_floating_point_v<T>) {
            rendered << std::setprecision(std::numeric_limits<T>::max_digits10) << actual;
        } else if constexpr (std::is_unsigned_v<T>) {
            rendered << static_cast<SQLUBIGINT>(actual);
        } else {
            rendered << static_cast<SQLBIGINT>(actual);
        }
        RecordProperty(std::string(prefix) + "_value", rendered.str());
        if constexpr (std::is_floating_point_v<T>) {
            const auto* value = std::get_if<SQLDOUBLE>(&expected);
            ASSERT_NE(nullptr, value);
            EXPECT_DOUBLE_EQ(*value, actual);
        } else if constexpr (std::is_unsigned_v<T>) {
            const auto* value = std::get_if<SQLUBIGINT>(&expected);
            ASSERT_NE(nullptr, value);
            EXPECT_EQ(*value, static_cast<SQLUBIGINT>(actual));
        } else {
            const auto* value = std::get_if<SQLBIGINT>(&expected);
            ASSERT_NE(nullptr, value);
            EXPECT_EQ(*value, static_cast<SQLBIGINT>(actual));
        }
    }

    void CheckValue(const ExpectedValue& expected, const char* prefix) {
        switch (GetParam().scenario.target) {
            case SQL_C_STINYINT: CheckScalar<signed char>(expected, prefix); break;
            case SQL_C_UTINYINT:
            case SQL_C_BIT: CheckScalar<SQLCHAR>(expected, prefix); break;
            case SQL_C_SSHORT: CheckScalar<SQLSMALLINT>(expected, prefix); break;
            case SQL_C_USHORT: CheckScalar<SQLUSMALLINT>(expected, prefix); break;
            case SQL_C_SLONG: CheckScalar<SQLINTEGER>(expected, prefix); break;
            case SQL_C_ULONG: CheckScalar<SQLUINTEGER>(expected, prefix); break;
            case SQL_C_SBIGINT: CheckScalar<SQLBIGINT>(expected, prefix); break;
            case SQL_C_UBIGINT: CheckScalar<SQLUBIGINT>(expected, prefix); break;
            case SQL_C_DOUBLE: CheckScalar<SQLDOUBLE>(expected, prefix); break;
            case SQL_C_NUMERIC: {
                SQL_NUMERIC_STRUCT numeric{};
                std::memcpy(&numeric, buffer_.data() + kOffset, sizeof(numeric));
                const auto* value = std::get_if<SQLUBIGINT>(&expected);
                ASSERT_NE(nullptr, value);
                EXPECT_EQ(1, numeric.sign);
                EXPECT_EQ(0, numeric.scale);
                EXPECT_GE(numeric.precision, 1);
                EXPECT_LE(numeric.precision, 38);
                RecordProperty(std::string(prefix) + "_numeric_precision",
                               static_cast<int>(numeric.precision));
                RecordProperty(std::string(prefix) + "_numeric_scale",
                               static_cast<int>(numeric.scale));
                for (std::size_t i = 0; i < SQL_MAX_NUMERIC_LEN; ++i) {
                    const auto byte = i < sizeof(*value) ? ((*value >> (8 * i)) & 0xFF) : 0;
                    EXPECT_EQ(byte, numeric.val[i]) << "numeric byte " << i;
                }
                break;
            }
            default: FAIL() << "Unhandled C target";
        }
    }

    void CheckRow(SQLRETURN expected_rc, const char* expected_state,
                  const ExpectedValue& expected_value, bool is_null, bool empty_delivery,
                  const char* prefix) {
        const auto width = TargetSize(GetParam().scenario.target);
        buffer_.fill(kGuard);
        indicator_.value = -999;
        SQLRETURN rc = SQLFetch(stmt_);
        if (!GetParam().bound) {
            ASSERT_EQ(SQL_SUCCESS, rc) << StmtDiagState();
            rc = SQLGetData(stmt_, 1, GetParam().scenario.target,
                            buffer_.data() + kOffset, width, &indicator_.value);
        }
        const std::string state = ODBCTestUtils::GetDiagState(SQL_HANDLE_STMT, stmt_);
        RecordProperty(std::string(prefix) + "_return", rc);
        RecordProperty(std::string(prefix) + "_sqlstate", state);
        RecordProperty(std::string(prefix) + "_diagnostics",
                       ODBCTestUtils::GetDiagMessage(SQL_HANDLE_STMT, stmt_));
        EXPECT_EQ(expected_rc, rc) << ODBCTestUtils::GetDiagMessage(SQL_HANDLE_STMT, stmt_);
        EXPECT_EQ(std::string(expected_state), state);
        CheckGuards(width);
        if (SQL_SUCCEEDED(rc)) {
            RecordProperty(std::string(prefix) + "_indicator", std::to_string(indicator_.value));
            if (is_null) {
                EXPECT_EQ(SQL_NULL_DATA, indicator_.value);
            } else if (empty_delivery) {
                EXPECT_EQ(0, indicator_.value);
                EXPECT_TRUE(std::all_of(buffer_.begin(), buffer_.end(),
                                        [](unsigned char byte) { return byte == kGuard; }));
            } else {
                EXPECT_EQ(width, indicator_.value);
                std::ostringstream bytes;
                bytes << std::hex << std::setfill('0');
                for (SQLLEN i = 0; i < width; ++i) {
                    bytes << std::setw(2)
                          << static_cast<unsigned>(buffer_[kOffset + static_cast<std::size_t>(i)]);
                }
                RecordProperty(std::string(prefix) + "_bytes", bytes.str());
                if (SQL_SUCCEEDED(expected_rc)) {
                    ASSERT_NO_FATAL_FAILURE(CheckValue(expected_value, prefix));
                }
            }
        }
    }
};

TEST_P(NumericParserConversionTest, ReferenceContract) {
    const auto& p = GetParam();
    const std::string declaration = std::string(p.wide ? "nvarchar(" : "varchar(") +
                                    std::to_string(p.scenario.column_size) + ")";
    if (p.scenario.literal) {
        ASSERT_LE(std::strlen(p.scenario.literal), p.scenario.column_size);
    }
    const std::string literal = p.scenario.expression ? p.scenario.expression
                                                      : QuoteLiteral(p.scenario.literal);
    // The server returns text; requesting a numeric C target makes the driver parse it.
    const std::string sql = "SELECT CAST(src.v AS " + declaration +
        ") AS value FROM (VALUES (1, " + literal +
        "), (2, '1')) AS src(ord, v) ORDER BY src.ord";
    ASSERT_NO_FATAL_FAILURE(ExecDirect(sql));
    SQLSMALLINT type = 0, scale = 0, nullable = 0;
    SQLULEN size = 0;
    ASSERT_SQL_OK(SQLDescribeCol(stmt_, 1, nullptr, 0, nullptr, &type, &size, &scale, &nullable),
                  SQL_HANDLE_STMT, stmt_);
    ASSERT_EQ(p.wide ? SQL_WVARCHAR : SQL_VARCHAR, type);
    ASSERT_EQ(p.scenario.column_size, size);
    if (p.bound) {
        ASSERT_SQL_OK(SQLBindCol(stmt_, 1, p.scenario.target, buffer_.data() + kOffset,
                                TargetSize(p.scenario.target), &indicator_.value),
                      SQL_HANDLE_STMT, stmt_);
    }
    ASSERT_NO_FATAL_FAILURE(CheckRow(p.scenario.result, p.scenario.state, p.scenario.value,
                                    p.scenario.literal == nullptr, p.scenario.empty_delivery,
                                    "first"));
    ExpectedValue one = SQLBIGINT{1};
    if (std::holds_alternative<SQLUBIGINT>(p.scenario.value)) {
        one = SQLUBIGINT{1};
    } else if (std::holds_alternative<SQLDOUBLE>(p.scenario.value)) {
        one = SQLDOUBLE{1};
    } else {
        switch (p.scenario.target) {
            case SQL_C_UTINYINT:
            case SQL_C_BIT:
            case SQL_C_USHORT:
            case SQL_C_ULONG:
            case SQL_C_UBIGINT:
            case SQL_C_NUMERIC: one = SQLUBIGINT{1}; break;
            case SQL_C_DOUBLE: one = SQLDOUBLE{1}; break;
            default: break;
        }
    }
    // Reuse the binding/column after the boundary case, including after conversion errors.
    ASSERT_NO_FATAL_FAILURE(CheckRow(SQL_SUCCESS, "", one, false, false, "recovery"));
    EXPECT_EQ(SQL_NO_DATA, SQLFetch(stmt_));
    ASSERT_SQL_OK(SQLFreeStmt(stmt_, SQL_UNBIND), SQL_HANDLE_STMT, stmt_);
    ASSERT_SQL_OK(SQLCloseCursor(stmt_), SQL_HANDLE_STMT, stmt_);
}

INSTANTIATE_TEST_SUITE_P(Shared, NumericParserConversionTest,
                        ::testing::ValuesIn(ParametersForAllPaths()), ParameterName);

}  // namespace
