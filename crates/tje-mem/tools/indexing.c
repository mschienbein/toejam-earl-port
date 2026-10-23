/* Synthetic MCC68K indexing probes: no ROM bytes or game asset content. */
struct index_record18 { char bytes[18]; };
struct index_record38 { char bytes[38]; };
struct index_record128 { char bytes[128]; };
struct index_record512 { char bytes[512]; };
unsigned long index_strides[] = {
    sizeof(char), sizeof(struct index_record18), sizeof(struct index_record38),
    sizeof(struct index_record128), sizeof(struct index_record512)
};

#define INDEX_PROBE(name, element, index_type) \
unsigned long name(base, index) \
element *base; index_type index; \
{ return (unsigned long)&base[index]; }

INDEX_PROBE(short_byte, char, short)
INDEX_PROBE(short_nonpow, struct index_record18, short)
INDEX_PROBE(short_pow, struct index_record128, short)
INDEX_PROBE(short_largepow, struct index_record512, short)
INDEX_PROBE(int_byte, char, int)
INDEX_PROBE(int_nonpow, struct index_record18, int)
INDEX_PROBE(int_pow, struct index_record128, int)
INDEX_PROBE(int_largepow, struct index_record512, int)
INDEX_PROBE(char_byte, char, char)
INDEX_PROBE(char_nonpow, struct index_record18, char)
INDEX_PROBE(char_pow, struct index_record128, char)
INDEX_PROBE(char_largepow, struct index_record512, char)
INDEX_PROBE(uchar_byte, char, unsigned char)
INDEX_PROBE(uchar_nonpow, struct index_record18, unsigned char)
INDEX_PROBE(uchar_pow, struct index_record128, unsigned char)
INDEX_PROBE(uchar_largepow, struct index_record512, unsigned char)
INDEX_PROBE(ushort_byte, char, unsigned short)
INDEX_PROBE(ushort_nonpow, struct index_record18, unsigned short)
INDEX_PROBE(ushort_pow, struct index_record128, unsigned short)
INDEX_PROBE(ushort_largepow, struct index_record512, unsigned short)

INDEX_PROBE(short_mul, struct index_record38, short)
INDEX_PROBE(int_mul, struct index_record38, int)
INDEX_PROBE(char_mul, struct index_record38, char)
INDEX_PROBE(uchar_mul, struct index_record38, unsigned char)
INDEX_PROBE(ushort_mul, struct index_record38, unsigned short)

extern char index_bytes[];
extern struct index_record18 index_nonpow[40];
extern struct index_record38 index_mul[40];
extern struct index_record128 index_pow[40];
extern struct index_record512 index_largepow[40];

#define BYTE_LOAD(name, index_type) \
unsigned long name(index) \
index_type index; \
{ return index_bytes[index]; }

#define RECORD_LOAD(name, array, index_type) \
unsigned long name(index) \
index_type index; \
{ return array[index].bytes[0]; }

BYTE_LOAD(load_short_byte, short)
RECORD_LOAD(load_short_nonpow, index_nonpow, short)
RECORD_LOAD(load_short_pow, index_pow, short)
RECORD_LOAD(load_short_largepow, index_largepow, short)
BYTE_LOAD(load_int_byte, int)
RECORD_LOAD(load_int_nonpow, index_nonpow, int)
RECORD_LOAD(load_int_pow, index_pow, int)
RECORD_LOAD(load_int_largepow, index_largepow, int)
BYTE_LOAD(load_char_byte, char)
RECORD_LOAD(load_char_nonpow, index_nonpow, char)
RECORD_LOAD(load_char_pow, index_pow, char)
RECORD_LOAD(load_char_largepow, index_largepow, char)
BYTE_LOAD(load_uchar_byte, unsigned char)
RECORD_LOAD(load_uchar_nonpow, index_nonpow, unsigned char)
RECORD_LOAD(load_uchar_pow, index_pow, unsigned char)
RECORD_LOAD(load_uchar_largepow, index_largepow, unsigned char)
BYTE_LOAD(load_ushort_byte, unsigned short)
RECORD_LOAD(load_ushort_nonpow, index_nonpow, unsigned short)
RECORD_LOAD(load_ushort_pow, index_pow, unsigned short)
RECORD_LOAD(load_ushort_largepow, index_largepow, unsigned short)

RECORD_LOAD(load_short_mul, index_mul, short)
RECORD_LOAD(load_int_mul, index_mul, int)
RECORD_LOAD(load_char_mul, index_mul, char)
RECORD_LOAD(load_uchar_mul, index_mul, unsigned char)
RECORD_LOAD(load_ushort_mul, index_mul, unsigned short)

#define GLOBAL_ADDRESS(name, element, array, index_type) \
element *name(index) \
index_type index; \
{ return &array[index]; }

GLOBAL_ADDRESS(global_short_byte, char, index_bytes, short)
GLOBAL_ADDRESS(global_short_nonpow, struct index_record18, index_nonpow, short)
GLOBAL_ADDRESS(global_short_mul, struct index_record38, index_mul, short)
GLOBAL_ADDRESS(global_short_pow, struct index_record128, index_pow, short)
GLOBAL_ADDRESS(global_short_largepow, struct index_record512, index_largepow, short)
GLOBAL_ADDRESS(global_int_byte, char, index_bytes, int)
GLOBAL_ADDRESS(global_int_nonpow, struct index_record18, index_nonpow, int)
GLOBAL_ADDRESS(global_int_mul, struct index_record38, index_mul, int)
GLOBAL_ADDRESS(global_int_pow, struct index_record128, index_pow, int)
GLOBAL_ADDRESS(global_int_largepow, struct index_record512, index_largepow, int)
GLOBAL_ADDRESS(global_char_byte, char, index_bytes, char)
GLOBAL_ADDRESS(global_char_nonpow, struct index_record18, index_nonpow, char)
GLOBAL_ADDRESS(global_char_mul, struct index_record38, index_mul, char)
GLOBAL_ADDRESS(global_char_pow, struct index_record128, index_pow, char)
GLOBAL_ADDRESS(global_char_largepow, struct index_record512, index_largepow, char)
GLOBAL_ADDRESS(global_uchar_byte, char, index_bytes, unsigned char)
GLOBAL_ADDRESS(global_uchar_nonpow, struct index_record18, index_nonpow, unsigned char)
GLOBAL_ADDRESS(global_uchar_mul, struct index_record38, index_mul, unsigned char)
GLOBAL_ADDRESS(global_uchar_pow, struct index_record128, index_pow, unsigned char)
GLOBAL_ADDRESS(global_uchar_largepow, struct index_record512, index_largepow, unsigned char)
GLOBAL_ADDRESS(global_ushort_byte, char, index_bytes, unsigned short)
GLOBAL_ADDRESS(global_ushort_nonpow, struct index_record18, index_nonpow, unsigned short)
GLOBAL_ADDRESS(global_ushort_mul, struct index_record38, index_mul, unsigned short)
GLOBAL_ADDRESS(global_ushort_pow, struct index_record128, index_pow, unsigned short)
GLOBAL_ADDRESS(global_ushort_largepow, struct index_record512, index_largepow, unsigned short)
