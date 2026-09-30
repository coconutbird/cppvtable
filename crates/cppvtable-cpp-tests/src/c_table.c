/* A C object and vtable, compiled as C rather than C++. */
#include <stdint.h>
#include <stdlib.h>

struct CValue;
struct CValueVtable {
    int64_t (*get)(struct CValue* self);
    void (*set)(struct CValue* self, int64_t value);
};
struct CValue {
    const struct CValueVtable* vtable;
    int64_t value;
};
static int64_t get(struct CValue* self) { return self->value; }
static void set(struct CValue* self, int64_t value) { self->value = value; }
static const struct CValueVtable vtable = {get, set};

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
