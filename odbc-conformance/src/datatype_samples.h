#pragma once

#include <sql.h>
#include <sqlext.h>
// The driver header's BCP macros collide with GoogleTest's assertion macros.
#pragma push_macro("FAIL")
#pragma push_macro("SUCCEED")
#undef FAIL
#undef SUCCEED
#include <msodbcsql.h>
#pragma pop_macro("SUCCEED")
#pragma pop_macro("FAIL")

#include <cstring>
#include <string>
#include <vector>

namespace odbc_samples {

using Bytes = std::vector<unsigned char>;

struct Sample {
    std::string name;
    std::string declaration;
    std::string expression;
    SQLSMALLINT c_type;
    Bytes native;
    std::string text;
    int scale = -1;
    bool generated = false;
    bool large = false;
};

template<class T>
Bytes bytes(const T& value)
{
    Bytes result(sizeof(T));
    std::memcpy(result.data(), &value, sizeof(T));
    return result;
}

inline Bytes narrow(const std::string& value)
{
    return Bytes(value.begin(), value.end());
}

inline Bytes wide(const std::string& value)
{
    std::vector<SQLWCHAR> characters(value.begin(), value.end());
    Bytes result(characters.size() * sizeof(SQLWCHAR));
    std::memcpy(result.data(), characters.data(), result.size());
    return result;
}

inline Bytes hex(const std::string& value)
{
    Bytes result;
    for (size_t i = 0; i < value.size(); i += 2) {
        result.push_back(static_cast<unsigned char>(std::stoul(value.substr(i, 2), nullptr, 16)));
    }
    return result;
}

inline Bytes numeric(const std::string& text, SQLSCHAR scale)
{
    SQL_NUMERIC_STRUCT value{};
    value.precision = 38;
    value.scale = scale;
    value.sign = text.front() == '-' ? 0 : 1;
    for (const char digit : text) {
        if (digit == '-' || digit == '.') {
            continue;
        }
        unsigned int carry = static_cast<unsigned int>(digit - '0');
        for (auto& octet : value.val) {
            carry += octet * 10U;
            octet = static_cast<SQLCHAR>(carry & 0xffU);
            carry >>= 8U;
        }
    }
    return bytes(value);
}

inline std::vector<Sample> all()
{
    const SQL_DATE_STRUCT date{2024, 2, 29};
    const SQL_TIMESTAMP_STRUCT datetime{2024, 2, 29, 12, 34, 56, 123000000};
    const SQL_TIMESTAMP_STRUCT datetime2{2024, 2, 29, 12, 34, 56, 123456700};
    const SQL_TIMESTAMP_STRUCT smalldatetime{2024, 2, 29, 12, 34, 0, 0};
    const SQL_SS_TIME2_STRUCT time{12, 34, 56, 123456700};
    const SQL_SS_TIMESTAMPOFFSET_STRUCT offset{2024, 2, 29, 12, 34, 56, 123456700, -5, -30};
    const SQLGUID guid{0x00112233, 0x4455, 0x6677,
                      {0x88, 0x99, 0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff}};
    const std::string guid_text = "00112233-4455-6677-8899-AABBCCDDEEFF";
    const std::string geometry = "00000000010C000000000000F03F0000000000000040";
    const std::string geography = "E6100000010C000000000000F03F0000000000000040";
    const std::string large_text(9000, 'x');
    const std::string large_wide(5000, 'w');
    const Bytes large_binary(9000, 0xab);
    std::string large_hex;
    for (size_t i = 0; i < large_binary.size(); ++i) {
        large_hex += "AB";
    }
    return {
        {"bigint", "bigint", "-9223372036854775807", SQL_C_SBIGINT,
         bytes<SQLBIGINT>(-9223372036854775807LL), "-9223372036854775807"},
        {"bit", "bit", "1", SQL_C_BIT, bytes<SQLCHAR>(1), "1"},
        {"tinyint", "tinyint", "255", SQL_C_UTINYINT, bytes<SQLCHAR>(255), "255"},
        {"smallint", "smallint", "-32768", SQL_C_SSHORT, bytes<SQLSMALLINT>(-32768), "-32768"},
        {"int", "int", "-2147483648", SQL_C_SLONG, bytes<SQLINTEGER>(-2147483647 - 1), "-2147483648"},
        {"decimal", "decimal(38,4)", "1234567890123456789012345678901234.5678", SQL_C_NUMERIC,
         numeric("1234567890123456789012345678901234.5678", 4),
         "1234567890123456789012345678901234.5678", 4},
        {"numeric", "numeric(18,5)", "-123456789.01234", SQL_C_NUMERIC,
         numeric("-123456789.01234", 5), "-123456789.01234", 5},
        {"money", "money", "123456.7890", SQL_C_NUMERIC,
         numeric("123456.7890", 4), "123456.7890", 4},
        {"smallmoney", "smallmoney", "-123.4500", SQL_C_NUMERIC,
         numeric("-123.4500", 4), "-123.4500", 4},
        {"float", "float(53)", "1.25", SQL_C_DOUBLE, bytes<SQLDOUBLE>(1.25), "1.25"},
        {"real", "real", "-2.5", SQL_C_FLOAT, bytes<SQLREAL>(-2.5F), "-2.5"},
        {"date", "date", "'2024-02-29'", SQL_C_TYPE_DATE, bytes(date), "2024-02-29"},
        {"datetime", "datetime", "'2024-02-29T12:34:56.123'", SQL_C_TYPE_TIMESTAMP,
         bytes(datetime), "2024-02-29 12:34:56.123"},
        {"datetime2", "datetime2(7)", "'2024-02-29T12:34:56.1234567'", SQL_C_TYPE_TIMESTAMP,
         bytes(datetime2), "2024-02-29 12:34:56.1234567"},
        {"smalldatetime", "smalldatetime", "'2024-02-29T12:34:00'", SQL_C_TYPE_TIMESTAMP,
         bytes(smalldatetime), "2024-02-29 12:34:00"},
        {"time", "time(7)", "'12:34:56.1234567'", SQL_C_BINARY,
         bytes(time), "12:34:56.1234567"},
        {"datetimeoffset", "datetimeoffset(7)", "'2024-02-29T12:34:56.1234567-05:30'", SQL_C_BINARY,
         bytes(offset), "2024-02-29 12:34:56.1234567 -05:30"},
        {"char", "char(8)", "'abc'", SQL_C_CHAR, narrow("abc     "), "abc     "},
        {"varchar", "varchar(50)", "'varchar value'", SQL_C_CHAR,
         narrow("varchar value"), "varchar value"},
        {"text", "text", "REPLICATE(CAST('x' AS varchar(max)),9000)", SQL_C_CHAR,
         narrow(large_text), large_text, -1, false, true},
        {"nchar", "nchar(8)", "N'abc'", SQL_C_WCHAR, wide("abc     "), "abc     "},
        {"nvarchar", "nvarchar(50)", "N'nvarchar value'", SQL_C_WCHAR,
         wide("nvarchar value"), "nvarchar value"},
        {"ntext", "ntext", "REPLICATE(CAST(N'w' AS nvarchar(max)),5000)", SQL_C_WCHAR,
         wide(large_wide), large_wide, -1, false, true},
        {"binary", "binary(5)", "0x0001AB", SQL_C_BINARY, hex("0001AB0000"), "0001AB0000"},
        {"varbinary", "varbinary(50)", "0x0001ABFF", SQL_C_BINARY, hex("0001ABFF"), "0001ABFF"},
        {"image", "image", "CONVERT(varbinary(max),REPLICATE(CAST('AB' AS varchar(max)),9000),2)",
         SQL_C_BINARY, large_binary, large_hex, -1, false, true},
        {"varchar_max", "varchar(max)", "REPLICATE(CAST('x' AS varchar(max)),9000)", SQL_C_CHAR,
         narrow(large_text), large_text, -1, false, true},
        {"nvarchar_max", "nvarchar(max)", "REPLICATE(CAST(N'w' AS nvarchar(max)),5000)", SQL_C_WCHAR,
         wide(large_wide), large_wide, -1, false, true},
        {"varbinary_max", "varbinary(max)",
         "CONVERT(varbinary(max),REPLICATE(CAST('AB' AS varchar(max)),9000),2)",
         SQL_C_BINARY, large_binary, large_hex, -1, false, true},
        {"uniqueidentifier", "uniqueidentifier", "'" + guid_text + "'", SQL_C_GUID,
         bytes(guid), guid_text},
        {"sql_variant", "sql_variant", "CAST(42 AS int)", SQL_C_SLONG, bytes<SQLINTEGER>(42), "42"},
        {"xml", "xml", "'<root><item>42</item></root>'", SQL_C_WCHAR,
         wide("<root><item>42</item></root>"), "<root><item>42</item></root>"},
        {"hierarchyid", "hierarchyid", "hierarchyid::Parse('/1/')", SQL_C_BINARY, hex("58"), "58"},
        {"geometry", "geometry", "geometry::Point(1,2,0)", SQL_C_BINARY, hex(geometry), geometry},
        {"geography", "geography", "geography::Point(1,2,4326)", SQL_C_BINARY, hex(geography), geography},
        {"sysname", "sysname", "N'odbc_name'", SQL_C_WCHAR, wide("odbc_name"), "odbc_name"},
        {"json", "json", "'{\"ok\":true}'", SQL_C_CHAR, narrow("{\"ok\":true}"), "{\"ok\":true}"},
        {"vector", "vector(3)", "'[1,2,3]'", SQL_C_CHAR, {}, "[1,2,3]"},
        {"rowversion", "rowversion", "", SQL_C_BINARY, {}, "", -1, true}
    };
}

} // namespace odbc_samples
