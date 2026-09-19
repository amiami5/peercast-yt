
#include "amf0.h"
#include "stream.h"

#include <iomanip> // setprecision
#include <sstream> // stringstream

namespace amf0 {

bool Deserializer::readBool(Stream &in)
{
    return in.readChar() != 0;
}

// 1 つの式に readChar() を複数書くと、評価順序は処理系依存 (GCC では逆順に
// なることがある)。また readChar() は char (符号付きのことがある) を返すので、
// 0x80 以上のバイトが符号拡張されて上位ビットを壊す。1 バイトずつ順に読み、
// 符号なしとして組み立てる。
int32_t Deserializer::readInt32(Stream &in)
{
    const uint32_t b0 = (uint8_t) in.readChar();
    const uint32_t b1 = (uint8_t) in.readChar();
    const uint32_t b2 = (uint8_t) in.readChar();
    const uint32_t b3 = (uint8_t) in.readChar();
    return (int32_t) ((b0 << 24) | (b1 << 16) | (b2 << 8) | b3);
}

int16_t Deserializer::readInt16(Stream& in)
{
    const uint16_t b0 = (uint8_t) in.readChar();
    const uint16_t b1 = (uint8_t) in.readChar();
    return (int16_t) (uint16_t) ((b0 << 8) | b1);
}

std::string Deserializer::readString(Stream &in)
{
    const int b0 = (uint8_t) in.readChar();
    const int b1 = (uint8_t) in.readChar();
    const int len = (b0 << 8) | b1;   // 0..65535
    return in.read(len);
}

double Deserializer::readDouble(Stream &in)
{
    double number;
    char* data = reinterpret_cast<char*>(&number);
    for (int i = 8; i > 0; i--) {
        char c = in.readChar();
        *(data + i - 1) = c;
    }
    return number;
}

std::vector<KeyValuePair> Deserializer::readObject(Stream &in)
{
    int budget = MAX_VALUES;
    return readObject(in, 0, budget);
}

Value Deserializer::readValue(Stream &in)
{
    int budget = MAX_VALUES;
    return readValue(in, 0, budget);
}

std::vector<KeyValuePair> Deserializer::readObject(Stream &in, int depth, int& budget)
{
    std::vector<KeyValuePair> list;
    while (true)
    {
        std::string key = readString(in);
        if (key == "")
            break;

        list.push_back({key, readValue(in, depth + 1, budget)});
    }
    in.readChar(); // OBJECT_END
    return list;
}

Value Deserializer::readValue(Stream &in, int depth, int& budget)
{
    if (depth > MAX_DEPTH)
        throw std::runtime_error("AMF0: nesting too deep");

    if (--budget < 0)
        throw std::runtime_error("AMF0: too many values");

    char type = in.readChar();

    switch (type)
    {
    case AMF_NUMBER:
    {
        return Value::number(readDouble(in));
    }
    case AMF_STRING:
    {
        return Value::string(readString(in));
    }
    case AMF_OBJECT:
    {
        return Value::object(readObject(in, depth, budget));
    }
    case AMF_BOOL:
    {
        return Value::boolean(readBool(in));
    }
    case AMF_ARRAY:
    {
        readInt32(in); // length
        return Value::array(readObject(in, depth, budget));
    }
    case AMF_STRICTARRAY:
    {
        int len = readInt32(in);
        std::vector<Value> list;
        for (int i = 0; i < len; i++) {
            list.push_back(readValue(in, depth + 1, budget));
        }
        return Value::strictArray(list);
    }
    case AMF_NULL:
    {
        return Value(nullptr);
    }
    case AMF_DATE:
    {
        double unixTime = readDouble(in);
        uint16_t timeZone = readInt16(in);
        return Value::date(unixTime, timeZone);
    }
    default:
        throw std::runtime_error("unknown AMF value type " + std::to_string(type));
    }
}

std::string Value::inspect() const
{
    switch (m_type)
    {
    case kNumber:
    {
        std::stringstream ss;
        ss << std::setprecision(std::numeric_limits<double>::max_digits10) << m_number;
        return ss.str();
    }
    case kObject:
    case kArray:
    {
        std::string buf = "{";
        bool first = true;
        for (auto pair : m_object)
        {
            if (!first)
                buf += ",";
            first = false;
            buf += string(pair.first).inspect() + ":" + pair.second.inspect();
        }
        buf += "}";
        return buf;
    }
    case kString:
        return str::json_inspect(m_string);
    case kNull:
        return "null";
    case kBool:
        return (m_bool) ? "true" : "false";
    case kDate:
        return "(" + std::to_string(m_date.unixTime) + ", " + std::to_string(m_date.timezone) + ")";
    case kStrictArray:
    {
        std::string buf = "[";
        bool first = true;
        for (auto elt : m_strict_array)
        {
            if (!first)
                buf += ",";
            first = false;
            buf += elt.inspect();
        }
        buf += "]";
        return buf;
    }
    default:
        throw std::runtime_error(str::format("inspect: unknown type %d", m_type));
    }
}

std::string format(const amf0::Value& value, int allowance, int indent)
{
    if (allowance <= 0 || value.inspect().size() <= allowance) {
        return value.inspect();
    } else if (value.isStrictArray()) {
        std::string out = "[\n";
        bool firstTime = true;
        for (const auto& elt : value.strictArray()) {
            if (!firstTime) {
                out += ",\n";
            }
            out += str::repeat(" ", indent + 2) + format(elt, allowance - indent, indent + 2);
            firstTime = false;
        }
        out += "\n" + str::repeat(" ", indent) + "]";
        return out;
    } else if (value.isObject() || value.isArray()) {
        std::string out = "{\n";
        bool firstTime = true;
        for (const auto& pair : value.object()) {
            if (!firstTime) {
                out += ",\n";
            }
            auto key = amf0::Value::string(pair.first).inspect();
            out += str::repeat(" ", indent + 2) + key + ": " + format(pair.second, allowance - indent - key.size() - 2 - 1, indent + 2);
            firstTime = false;
        }
        out += "\n" + str::repeat(" ", indent) + "}";
        return out;
    } else {
        return value.inspect();
    }
}

} // namespace amf0
