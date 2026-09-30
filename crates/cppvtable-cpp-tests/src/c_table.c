/* A C object and vtable, compiled as C rather than C++. */
#include <stdint.h>
#include <stdlib.h>

struct CValue;
struct CValueVtable {
    int64_t (*get)(struct CValue* self);
    void (*set)(struct CValue* self, int64_t value);
    int64_t (*read)(struct CValue* self, const int64_t* input);
    void (*write)(struct CValue* self, int64_t* output);
};
struct CValue {
    const struct CValueVtable* vtable;
    int64_t value;
};
static int64_t get(struct CValue* self) { return self->value; }
static void set(struct CValue* self, int64_t value) { self->value = value; }
static int64_t read_value(struct CValue* self, const int64_t* input) { (void)self; return *input; }
static void write_value(struct CValue* self, int64_t* output) { *output = self->value; }
static const struct CValueVtable vtable = {get, set, read_value, write_value};

void* cppvtable_c_create(int64_t value) {
    struct CValue* object = (struct CValue*)malloc(sizeof(struct CValue));
    if (object) { object->vtable = &vtable; object->value = value; }
    return object;
}
void cppvtable_c_delete(void* object) { free(object); }
int64_t cppvtable_c_get(void* object) {
    struct CValue* self = (struct CValue*)object;
    return self->vtable->get(self);
}
void cppvtable_c_set(void* object, int64_t value) {
    struct CValue* self = (struct CValue*)object;
    self->vtable->set(self, value);
}
int64_t cppvtable_c_read(void* object, const int64_t* input) {
    struct CValue* self = (struct CValue*)object;
    return self->vtable->read(self, input);
}
void cppvtable_c_write(void* object, int64_t* output) {
    struct CValue* self = (struct CValue*)object;
    self->vtable->write(self, output);
}

/* Distinct per-function conventions must retain their stack/register rules on x86. */
#if defined(_WIN32) && (defined(_M_IX86) || defined(__i386__))
#define CPPVTABLE_CDECL __cdecl
#define CPPVTABLE_STDCALL __stdcall
#define CPPVTABLE_FASTCALL __fastcall
#else
#define CPPVTABLE_CDECL
#define CPPVTABLE_STDCALL
#define CPPVTABLE_FASTCALL
#endif
struct CConventions;
struct CConventionsVtable {
    int32_t (CPPVTABLE_CDECL *plain)(struct CConventions* self, int32_t value);
    int32_t (CPPVTABLE_STDCALL *system_call)(struct CConventions* self, int32_t value);
    int32_t (CPPVTABLE_FASTCALL *fast_call)(struct CConventions* self, int32_t value);
};
struct CConventions {
    const struct CConventionsVtable* vtable;
    int32_t value;
};
static int32_t CPPVTABLE_CDECL plain(struct CConventions* self, int32_t value) {
    return self->value + value;
}
static int32_t CPPVTABLE_STDCALL system_call(struct CConventions* self, int32_t value) {
    return self->value + 2 * value;
}
static int32_t CPPVTABLE_FASTCALL fast_call(struct CConventions* self, int32_t value) {
    return self->value + 3 * value;
}
static const struct CConventionsVtable conventions_vtable = {
    plain, system_call, fast_call
};
void* cppvtable_c_conventions_create(int32_t value) {
    struct CConventions* object = (struct CConventions*)malloc(sizeof(struct CConventions));
    if (object) { object->vtable = &conventions_vtable; object->value = value; }
    return object;
}
int32_t cppvtable_c_conventions_cdecl(void* object, int32_t value) {
    struct CConventions* self = (struct CConventions*)object;
    return self->vtable->plain(self, value);
}
int32_t cppvtable_c_conventions_stdcall(void* object, int32_t value) {
    struct CConventions* self = (struct CConventions*)object;
    return self->vtable->system_call(self, value);
}
int32_t cppvtable_c_conventions_fastcall(void* object, int32_t value) {
    struct CConventions* self = (struct CConventions*)object;
    return self->vtable->fast_call(self, value);
}

/* Only entry 32 is declared; all 50 entries are still present in the native table. */
struct CPartial;
typedef int32_t (*CPartialMethod)(struct CPartial* self, int32_t value);
struct CPartialVtable {
    CPartialMethod before[32];
    CPartialMethod known;
    CPartialMethod after[17];
};
struct CPartial {
    const struct CPartialVtable* vtable;
    int32_t value;
};
static int32_t partial_known(struct CPartial* self, int32_t value) {
    return self->value + value;
}
static const struct CPartialVtable partial_vtable = {{0}, partial_known, {0}};
void* cppvtable_c_partial_create(int32_t value) {
    struct CPartial* object = (struct CPartial*)malloc(sizeof(struct CPartial));
    if (object) { object->vtable = &partial_vtable; object->value = value; }
    return object;
}
int32_t cppvtable_c_partial_call(void* object, int32_t value) {
    struct CPartial* self = (struct CPartial*)object;
    return self->vtable->known(self, value);
}
