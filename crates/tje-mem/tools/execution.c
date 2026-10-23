/* Synthetic MCC68K execution probes. No cartridge bytes or game assets. */
unsigned long expression_quotient(dividend, divisor)
unsigned long dividend, divisor;
{
    return dividend / divisor;
}
unsigned long expression_remainder(dividend, divisor)
unsigned long dividend, divisor;
{
    return dividend % divisor;
}
struct aggregate_probe {
    char first;
    long value;
    char last;
};
unsigned long aggregate_layout[] = {
    sizeof(struct aggregate_probe),
    (unsigned long)&((struct aggregate_probe *)0)->first,
    (unsigned long)&((struct aggregate_probe *)0)->value,
    (unsigned long)&((struct aggregate_probe *)0)->last
};
void copy_record(destination, source)
struct aggregate_probe *destination, *source;
{
    *destination = *source;
}
long record_argument(record)
struct aggregate_probe record;
{
    return record.first + record.value + record.last;
}
long call_record_argument(record)
struct aggregate_probe *record;
{
    return record_argument(*record);
}
struct aggregate_probe record_return(record)
struct aggregate_probe *record;
{
    return *record;
}
long compare_pointers(left, right)
char *left, *right;
{
    return left < right;
}
long subtract_short_pointers(left, right)
short *left, *right;
{
    return left - right;
}
long post_char_pointer(pointer)
char **pointer;
{
    return *(*pointer)++;
}
long post_short_pointer(pointer)
short **pointer;
{
    return *(*pointer)++;
}
void compound_char(pointer, delta)
char *pointer;
long delta;
{
    *pointer += delta;
}
void compound_short(pointer, delta)
short *pointer;
long delta;
{
    *pointer += delta;
}
long post_char(pointer)
char *pointer;
{
    return (*pointer)++;
}
long post_short(pointer)
short *pointer;
{
    return (*pointer)++;
}
