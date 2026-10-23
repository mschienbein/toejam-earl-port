/* Synthetic compiler probes, not game code or ROM content. K&R definitions are intentional. */
unsigned long semantic_values[] = {
    sizeof(char), sizeof(short), sizeof(int), sizeof(long), sizeof(void *),
    sizeof(float), sizeof(double),
    (long)(char)-1, (unsigned long)(-2147483647L - 1),
    -7L / 3L, -7L % 3L, -7L >> 2,
    (unsigned short)65535 + 1,
    (unsigned long)-1,
    (char)-1 < 0, (unsigned short)65535 < -1,
    (unsigned long)1 < -1L, -1L < (unsigned long)1
};
long promote_char(value)
char value;
{
    return value;
}
long promote_unsigned_char(value)
unsigned char value;
{
    return value;
}
long promote_short(value)
short value;
{
    return value + 1;
}
long promote_unsigned_short(value)
unsigned short value;
{
    return value + 1;
}
long shift_signed(value, count)
long value; int count;
{
    return value >> count;
}
unsigned long shift_unsigned(value, count)
unsigned long value; int count;
{
    return value >> count;
}
int compare_mixed(value, bound)
long value; unsigned long bound;
{
    return value < bound;
}
int compare_unsigned_short(value, bound)
unsigned short value; int bound;
{
    return value < bound;
}
int compare_unsigned_char(value, bound)
unsigned char value; int bound;
{
    return value < bound;
}
long signed_divide(value, divisor)
long value, divisor;
{
    return value / divisor;
}
long signed_remainder(value, divisor)
long value, divisor;
{
    return value % divisor;
}
unsigned long unsigned_divide(value, divisor)
unsigned long value, divisor;
{
    return value / divisor;
}
unsigned long unsigned_remainder(value, divisor)
unsigned long value, divisor;
{
    return value % divisor;
}
int short_circuit()
{
    extern long first(); extern long second();
    return first() && second();
}
long comma_order()
{
    extern long first(); extern long second();
    return first(), second();
}
long call_order()
{
    extern long first(); extern long second(); extern long consume();
    return consume(first(), second());
}
long sum_order()
{
    extern long first(); extern long second();
    return first() + second();
}
void store_short(value)
short *value;
{
    ++*value;
}
long switch_fallthrough(value)
long value;
{
    long result = 0;
    switch (value) {
    case 1:
        ++result;
    case 2:
        result += 2;
        break;
    default:
        result = -1;
    }
    return result;
}
long persistent_counter()
{
    static long counter = 7;
    return counter++;
}
