/* C function tables embedded directly in each object, without a table pointer. */
#include <stdint.h>
#include <stdlib.h>

struct CInlineBase {
    int64_t (*get)(void* self);
    void (*set)(void* self, int64_t value);
};
struct CInlineDerived {
    struct CInlineBase base;
    void (*write)(void* self, int64_t* output);
    int64_t value;
};
static int64_t inline_get(void* self) { return ((struct CInlineDerived*)self)->value; }
static void inline_set(void* self, int64_t value) { ((struct CInlineDerived*)self)->value = value; }
static void inline_write(void* self, int64_t* output) { *output = ((struct CInlineDerived*)self)->value; }

void* cppvtable_c_inline_create(int64_t value) {
    struct CInlineDerived* object = (struct CInlineDerived*)malloc(sizeof(struct CInlineDerived));
    if (object) {
        object->base.get = inline_get;
        object->base.set = inline_set;
        object->write = inline_write;
        object->value = value;
    }
    return object;
}
void cppvtable_c_inline_delete(void* object) { free(object); }
int64_t cppvtable_c_inline_get(void* object) {
    return ((struct CInlineBase*)object)->get(object);
}
void cppvtable_c_inline_set(void* object, int64_t value) {
    ((struct CInlineBase*)object)->set(object, value);
}
void cppvtable_c_inline_write(void* object, int64_t* output) {
    ((struct CInlineDerived*)object)->write(object, output);
}
int64_t cppvtable_c_inline_native_state(void* object) {
    return ((struct CInlineDerived*)object)->value;
}

#if defined(_WIN32) && (defined(_M_IX86) || defined(__i386__))
#define CPPVTABLE_INLINE_SYSTEM __stdcall
#define CPPVTABLE_INLINE_FAST __fastcall
#else
#define CPPVTABLE_INLINE_SYSTEM
#define CPPVTABLE_INLINE_FAST
#endif
struct CInlinePartial;
typedef void (*CInlineReserved)(void);
struct CInlinePartial {
    CInlineReserved before[32];
    int32_t (CPPVTABLE_INLINE_SYSTEM *known)(void* self, int32_t increment);
    void (CPPVTABLE_INLINE_FAST *update)(void* self, int32_t value);
    CInlineReserved after[16];
    int32_t value;
};
static int32_t CPPVTABLE_INLINE_SYSTEM inline_partial_known(void* self, int32_t increment) {
    return ((struct CInlinePartial*)self)->value + increment;
}
static void CPPVTABLE_INLINE_FAST inline_partial_update(void* self, int32_t value) {
    ((struct CInlinePartial*)self)->value = value;
}
void* cppvtable_c_inline_partial_create(int32_t value) {
    struct CInlinePartial* object = (struct CInlinePartial*)calloc(1, sizeof(struct CInlinePartial));
    if (object) {
        object->known = inline_partial_known;
        object->update = inline_partial_update;
        object->value = value;
    }
    return object;
}
int32_t cppvtable_c_inline_partial_call(void* object, int32_t increment) {
    return ((struct CInlinePartial*)object)->known(object, increment);
}
void cppvtable_c_inline_partial_update(void* object, int32_t value) {
    ((struct CInlinePartial*)object)->update(object, value);
}
int32_t cppvtable_c_inline_partial_native_state(void* object) {
    return ((struct CInlinePartial*)object)->value;
}

struct CInlinePointerViewTable {
    int64_t (*get)(void* self);
};
struct CInlinePointerView {
    const struct CInlinePointerViewTable* table;
};
int64_t cppvtable_c_inline_pointer_view_get(void* object) {
    return ((struct CInlinePointerView*)object)->table->get(object);
}
