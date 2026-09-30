//! An interface chain of depth 3, the way Direct3D 9 builds one:
//! `ITexture` : `IBaseTexture` : `IResource` : `IUnknown`.
//!
//! The tests check the vtable layout of the whole chain, the ancestor list,
//! `QueryInterface` for each ancestor, and the `Deref` chain that gives the methods of
//! each base interface.

use core::mem::{offset_of, size_of};
use core::ptr;
use core::sync::atomic::{AtomicU32, Ordering};

use cppvtable::{
    ComObject, ComPtr, E_POINTER, HRESULT, IUnknown, Interface, RefCounted, S_OK, SingleRefCount,
    implement, interface,
};

/// The root of the chain.
#[interface(abi = com, iid = "3b0f0001-0000-4000-8000-000000000001")]
pub unsafe trait IResource {
    /// Set the priority and give the old one.
    fn SetPriority(&self, priority: u32) -> u32;
    /// Give the priority.
    fn GetPriority(&self) -> u32;
}

/// The middle of the chain.
#[interface(abi = com, iid = "3b0f0002-0000-4000-8000-000000000002", extends(IResource))]
pub unsafe trait IBaseTexture {
    /// Give the number of levels.
    fn GetLevelCount(&self) -> u32;
}

/// The leaf of the chain.
#[interface(abi = com, iid = "3b0f0003-0000-4000-8000-000000000003", extends(IBaseTexture))]
pub unsafe trait ITexture {
    /// Write the width of the level to `width`.
    fn GetLevelWidth(&self, level: u32, width: *mut u32) -> HRESULT;
}

/// The object of the chain.
#[implement(ITexture)]
struct Texture {
    /// The priority of the resource.
    priority: AtomicU32,
    /// The number of levels.
    levels: u32,
}

impl RefCounted for Texture {
    type Policy = SingleRefCount;
}

impl IResourceImpl for Texture {
    fn SetPriority(&self, priority: u32) -> u32 {
        self.priority.swap(priority, Ordering::Relaxed)
    }

    fn GetPriority(&self) -> u32 {
        self.priority.load(Ordering::Relaxed)
    }
}

impl IBaseTextureImpl for Texture {
    fn GetLevelCount(&self) -> u32 {
        self.levels
    }
}

impl ITextureImpl for Texture {
    fn GetLevelWidth(&self, level: u32, width: *mut u32) -> HRESULT {
        if width.is_null() || level >= self.levels {
            return E_POINTER;
        }
        // SAFETY: The pointer is not null, and the caller gives a writable place.
        unsafe { *width = 256 >> level };
        S_OK
    }
}

/// Make a new texture with 4 levels.
fn new_texture() -> ComPtr<ITexture> {
    ComObject::new(Texture {
        priority: AtomicU32::new(0),
        levels: 4,
    })
}

#[test]
fn the_vtable_of_the_chain_grows_at_the_end() {
    let pointer = size_of::<usize>();
    assert_eq!(size_of::<IResourceVtbl>(), 5 * pointer);
    assert_eq!(size_of::<IBaseTextureVtbl>(), 6 * pointer);
    assert_eq!(size_of::<ITextureVtbl>(), 7 * pointer);

    assert_eq!(offset_of!(IResourceVtbl, base), 0);
    assert_eq!(offset_of!(IResourceVtbl, SetPriority), 3 * pointer);
    assert_eq!(offset_of!(IResourceVtbl, GetPriority), 4 * pointer);

    assert_eq!(offset_of!(IBaseTextureVtbl, base), 0);
    assert_eq!(offset_of!(IBaseTextureVtbl, GetLevelCount), 5 * pointer);

    assert_eq!(offset_of!(ITextureVtbl, base), 0);
    assert_eq!(offset_of!(ITextureVtbl, GetLevelWidth), 6 * pointer);
}

#[test]
fn the_ancestor_list_holds_the_whole_chain_in_order() {
    assert_eq!(IResource::ANCESTORS, &[IUnknown::IID]);
    assert_eq!(IBaseTexture::ANCESTORS, &[IResource::IID, IUnknown::IID]);
    assert_eq!(
        ITexture::ANCESTORS,
        &[IBaseTexture::IID, IResource::IID, IUnknown::IID]
    );
}

#[test]
fn a_c_caller_reaches_a_method_of_each_level_of_the_chain() {
    let texture = new_texture();
    let this = texture.as_raw();
    // SAFETY: `this` is a valid interface pointer of `ITexture`.
    let vtable = unsafe { *this.cast::<*const ITextureVtbl>() };

    // SAFETY: The vtable belongs to the object.
    assert_eq!(unsafe { ((*vtable).base.base.GetPriority)(this) }, 0);
    // SAFETY: The vtable belongs to the object.
    assert_eq!(unsafe { ((*vtable).base.base.SetPriority)(this, 7) }, 0);
    // SAFETY: The vtable belongs to the object.
    assert_eq!(unsafe { ((*vtable).base.base.GetPriority)(this) }, 7);
    // SAFETY: The vtable belongs to the object.
    assert_eq!(unsafe { ((*vtable).base.GetLevelCount)(this) }, 4);

    let mut width = 0_u32;
    // SAFETY: The vtable belongs to the object and `width` is a local value.
    let result = unsafe { ((*vtable).GetLevelWidth)(this, 2, &raw mut width) };
    assert!(result.is_ok());
    assert_eq!(width, 64);

    // The three `IUnknown` slots are at the start of the whole chain.
    // SAFETY: The vtable belongs to the object and this call owns a reference.
    assert_eq!(unsafe { ((*vtable).base.base.base.AddRef)(this) }, 2);
    // SAFETY: This call removes the reference of the line above.
    assert_eq!(unsafe { ((*vtable).base.base.base.Release)(this) }, 1);
}

#[test]
fn the_deref_chain_gives_the_methods_of_each_base_interface() {
    let texture = new_texture();
    // `ComPtr<ITexture>` derefs to `ITexture`, then to `IBaseTexture`, then to
    // `IResource`, then to `IUnknown`.
    // SAFETY: The object is alive.
    unsafe {
        assert_eq!(texture.GetLevelCount(), 4);
        assert_eq!(texture.SetPriority(3), 0);
        assert_eq!(texture.GetPriority(), 3);
        assert_eq!(texture.AddRef(), 2);
        assert_eq!(texture.Release(), 1);
    }
}

#[test]
fn query_interface_answers_every_ancestor_with_the_same_pointer() {
    let texture = new_texture();
    let this = texture.as_raw();

    let base: ComPtr<IBaseTexture> = texture.cast().unwrap();
    let resource: ComPtr<IResource> = texture.cast().unwrap();
    let unknown: ComPtr<IUnknown> = texture.cast().unwrap();
    assert_eq!(base.as_raw(), this);
    assert_eq!(resource.as_raw(), this);
    assert_eq!(unknown.as_raw(), this);
    assert_eq!(texture.public_count_of::<Texture>(), Some(4));

    // A cast from the middle of the chain works as well.
    let leaf: ComPtr<ITexture> = resource.cast().unwrap();
    assert_eq!(leaf.as_raw(), this);

    // SAFETY: The object is alive.
    assert_eq!(unsafe { resource.GetPriority() }, 0);
}

#[test]
fn a_pointer_of_a_base_interface_calls_the_method_of_the_object() {
    let texture = new_texture();
    let resource: ComPtr<IResource> = texture.cast().unwrap();
    // SAFETY: The object is alive.
    unsafe { resource.SetPriority(11) };

    // The interface reference of a borrowed raw pointer gives the same answer.
    let raw = texture.as_raw();
    // SAFETY: The place holds a valid interface pointer of a live object.
    let borrowed = unsafe { IResource::from_raw_ref(&raw) };
    // SAFETY: The object is alive.
    assert_eq!(unsafe { borrowed.GetPriority() }, 11);
    assert_eq!(borrowed.as_raw(), raw);
    assert!(ptr::eq(
        borrowed.vtable().cast::<u8>(),
        texture.vtable().cast::<u8>()
    ));
}
