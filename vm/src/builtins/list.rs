use super::{PositionIterInternal, PyGenericAlias, PyTupleRef, PyType, PyTypeRef};
use crate::atomic_func;
use crate::common::lock::{
    PyMappedRwLockReadGuard, PyMutex, PyRwLock, PyRwLockReadGuard, PyRwLockWriteGuard,
};
use crate::{
    AsObject, Context, Py, PyObject, PyObjectRef, PyPayload, PyRef, PyResult,
    class::PyClassImpl,
    convert::ToPyObject,
    function::{ArgSize, FuncArgs, OptionalArg, PyComparisonValue},
    iter::PyExactSizeIterator,
    protocol::{PyIterReturn, PyMappingMethods, PySequenceMethods},
    recursion::ReprGuard,
    sequence::{MutObjectSequenceOp, OptionalRangeArgs, SequenceExt, SequenceMutExt},
    sliceable::{SequenceIndex, SliceableSequenceMutOp, SliceableSequenceOp},
    types::{
        AsMapping, AsSequence, Comparable, Constructor, Initializer, IterNext, Iterable,
        PyComparisonOp, Representable, SelfIter, Unconstructible,
    },
    utils::collection_repr,
    vm::VirtualMachine,
};
use std::{fmt, ops::DerefMut};

#[pyclass(module = false, name = "list", unhashable = true, traverse)]
#[derive(Default)]
pub struct PyList {
    elements: PyRwLock<Vec<PyObjectRef>>,
}

impl fmt::Debug for PyList {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // TODO: implement more detailed, non-recursive Debug formatter
        f.write_str("list")
    }
}

impl From<Vec<PyObjectRef>> for PyList {
    fn from(elements: Vec<PyObjectRef>) -> Self {
        Self {
            elements: PyRwLock::new(elements),
        }
    }
}

impl FromIterator<PyObjectRef> for PyList {
    fn from_iter<T: IntoIterator<Item = PyObjectRef>>(iter: T) -> Self {
        Vec::from_iter(iter).into()
    }
}

impl PyPayload for PyList {
    #[inline]
    fn class(ctx: &Context) -> &'static Py<PyType> {
        ctx.types.list_type
    }
}

impl ToPyObject for Vec<PyObjectRef> {
    fn to_pyobject(self, vm: &VirtualMachine) -> PyObjectRef {
        PyList::from(self).into_ref(&vm.ctx).into()
    }
}

impl PyList {
    pub fn new_ref(elements: Vec<PyObjectRef>, ctx: &Context) -> PyRef<Self> {
        PyRef::new_ref(Self::from(elements), ctx.types.list_type.to_owned(), None)
    }

    pub fn borrow_vec(&self) -> PyMappedRwLockReadGuard<'_, [PyObjectRef]> {
        PyRwLockReadGuard::map(self.elements.read(), |v| &**v)
    }

    pub fn borrow_vec_mut(&self) -> PyRwLockWriteGuard<'_, Vec<PyObjectRef>> {
        self.elements.write()
    }

    fn repeat(&self, n: isize, vm: &VirtualMachine) -> PyResult<PyRef<Self>> {
        let elements = &*self.borrow_vec();
        let v = elements.mul(vm, n)?;
        Ok(Self::from(v).into_ref(&vm.ctx))
    }

    fn irepeat(zelf: PyRef<Self>, n: isize, vm: &VirtualMachine) -> PyResult<PyRef<Self>> {
        zelf.borrow_vec_mut().imul(vm, n)?;
        Ok(zelf)
    }
}

#[derive(FromArgs, Default, Traverse)]
pub(crate) struct SortOptions {
    #[pyarg(named, default)]
    key: Option<PyObjectRef>,
    #[pytraverse(skip)]
    #[pyarg(named, default = false)]
    reverse: bool,
}

pub type PyListRef = PyRef<PyList>;

#[pyclass(
    with(
        Constructor,
        Initializer,
        AsMapping,
        Iterable,
        Comparable,
        AsSequence,
        Representable
    ),
    flags(BASETYPE)
)]
impl PyList {
    #[pymethod]
    pub(crate) fn append(&self, x: PyObjectRef) {
        self.borrow_vec_mut().push(x);
    }

    #[pymethod]
    pub(crate) fn extend(&self, x: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        let mut new_elements = x.try_to_value(vm)?;
        self.borrow_vec_mut().append(&mut new_elements);
        Ok(())
    }

    #[pymethod]
    pub(crate) fn insert(&self, position: isize, element: PyObjectRef) {
        let mut elements = self.borrow_vec_mut();
        let position = elements.saturate_index(position);
        elements.insert(position, element);
    }

    fn concat(&self, other: &PyObject, vm: &VirtualMachine) -> PyResult<PyRef<Self>> {
        let other = other.downcast_ref::<Self>().ok_or_else(|| {
            vm.new_type_error(format!(
                "Cannot add {} and {}",
                Self::class(&vm.ctx).name(),
                other.class().name()
            ))
        })?;
        let mut elements = self.borrow_vec().to_vec();
        elements.extend(other.borrow_vec().iter().cloned());
        Ok(Self::from(elements).into_ref(&vm.ctx))
    }

    #[pymethod]
    fn __add__(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyRef<Self>> {
        self.concat(&other, vm)
    }

    fn inplace_concat(
        zelf: &Py<Self>,
        other: &PyObject,
        vm: &VirtualMachine,
    ) -> PyResult<PyObjectRef> {
        let mut seq = extract_cloned(other, Ok, vm)?;
        zelf.borrow_vec_mut().append(&mut seq);
        Ok(zelf.to_owned().into())
    }

    #[pymethod]
    fn __iadd__(
        zelf: PyRef<Self>,
        other: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<PyRef<Self>> {
        let mut seq = extract_cloned(&other, Ok, vm)?;
        zelf.borrow_vec_mut().append(&mut seq);
        Ok(zelf)
    }

    #[pymethod]
    fn clear(&self) {
        let _removed = std::mem::take(self.borrow_vec_mut().deref_mut());
    }

    #[pymethod]
    fn copy(&self, vm: &VirtualMachine) -> PyRef<Self> {
        Self::from(self.borrow_vec().to_vec()).into_ref(&vm.ctx)
    }

    #[allow(clippy::len_without_is_empty)]
    #[pymethod]
    pub fn __len__(&self) -> usize {
        self.borrow_vec().len()
    }

    #[pymethod]
    fn __sizeof__(&self) -> usize {
        std::mem::size_of::<Self>()
            + self.elements.read().capacity() * std::mem::size_of::<PyObjectRef>()
    }

    #[pymethod]
    fn reverse(&self) {
        self.borrow_vec_mut().reverse();
    }

    #[pymethod]
    fn __reversed__(zelf: PyRef<Self>) -> PyListReverseIterator {
        let position = zelf.__len__().saturating_sub(1);
        PyListReverseIterator {
            internal: PyMutex::new(PositionIterInternal::new(zelf, position)),
        }
    }

    fn _getitem(&self, needle: &PyObject, vm: &VirtualMachine) -> PyResult {
        match SequenceIndex::try_from_borrowed_object(vm, needle, "list")? {
            SequenceIndex::Int(i) => self.borrow_vec().getitem_by_index(vm, i),
            SequenceIndex::Slice(slice) => self
                .borrow_vec()
                .getitem_by_slice(vm, slice)
                .map(|x| vm.ctx.new_list(x).into()),
        }
    }

    #[pymethod]
    fn __getitem__(&self, needle: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        self._getitem(&needle, vm)
    }

    fn _setitem(&self, needle: &PyObject, value: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        match SequenceIndex::try_from_borrowed_object(vm, needle, "list")? {
            SequenceIndex::Int(index) => self.borrow_vec_mut().setitem_by_index(vm, index, value),
            SequenceIndex::Slice(slice) => {
                let sec = extract_cloned(&value, Ok, vm)?;
                self.borrow_vec_mut().setitem_by_slice(vm, slice, &sec)
            }
        }
    }

    #[pymethod]
    fn __setitem__(
        &self,
        needle: PyObjectRef,
        value: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        self._setitem(&needle, value, vm)
    }

    #[pymethod]
    #[pymethod(name = "__rmul__")]
    fn __mul__(&self, n: ArgSize, vm: &VirtualMachine) -> PyResult<PyRef<Self>> {
        self.repeat(n.into(), vm)
    }

    #[pymethod]
    fn __imul__(zelf: PyRef<Self>, n: ArgSize, vm: &VirtualMachine) -> PyResult<PyRef<Self>> {
        Self::irepeat(zelf, n.into(), vm)
    }

    #[pymethod]
    fn count(&self, needle: PyObjectRef, vm: &VirtualMachine) -> PyResult<usize> {
        self.mut_count(vm, &needle)
    }

    #[pymethod]
    pub(crate) fn __contains__(&self, needle: PyObjectRef, vm: &VirtualMachine) -> PyResult<bool> {
        self.mut_contains(vm, &needle)
    }

    #[pymethod]
    fn index(
        &self,
        needle: PyObjectRef,
        range: OptionalRangeArgs,
        vm: &VirtualMachine,
    ) -> PyResult<usize> {
        let (start, stop) = range.saturate(self.__len__(), vm)?;
        let index = self.mut_index_range(vm, &needle, start..stop)?;
        if let Some(index) = index.into() {
            Ok(index)
        } else {
            Err(vm.new_value_error(format!("'{}' is not in list", needle.str(vm)?)))
        }
    }

    #[pymethod]
    fn pop(&self, i: OptionalArg<isize>, vm: &VirtualMachine) -> PyResult {
        let mut i = i.into_option().unwrap_or(-1);
        let mut elements = self.borrow_vec_mut();
        if i < 0 {
            i += elements.len() as isize;
        }
        if elements.is_empty() {
            Err(vm.new_index_error("pop from empty list"))
        } else if i < 0 || i as usize >= elements.len() {
            Err(vm.new_index_error("pop index out of range"))
        } else {
            Ok(elements.remove(i as usize))
        }
    }

    #[pymethod]
    fn remove(&self, needle: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        let index = self.mut_index(vm, &needle)?;

        if let Some(index) = index.into() {
            // defer delete out of borrow
            let is_inside_range = index < self.borrow_vec().len();
            Ok(is_inside_range.then(|| self.borrow_vec_mut().remove(index)))
        } else {
            Err(vm.new_value_error(format!("'{}' is not in list", needle.str(vm)?)))
        }
        .map(drop)
    }

    fn _delitem(&self, needle: &PyObject, vm: &VirtualMachine) -> PyResult<()> {
        match SequenceIndex::try_from_borrowed_object(vm, needle, "list")? {
            SequenceIndex::Int(i) => self.borrow_vec_mut().delitem_by_index(vm, i),
            SequenceIndex::Slice(slice) => self.borrow_vec_mut().delitem_by_slice(vm, slice),
        }
    }

    #[pymethod]
    fn __delitem__(&self, subscript: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        self._delitem(&subscript, vm)
    }

    #[pymethod]
    pub(crate) fn sort(&self, options: SortOptions, vm: &VirtualMachine) -> PyResult<()> {
        // replace list contents with [] for duration of sort.
        // this prevents keyfunc from messing with the list and makes it easy to
        // check if it tries to append elements to it.
        let mut elements = std::mem::take(self.borrow_vec_mut().deref_mut());
        let res = do_sort(vm, &mut elements, options.key, options.reverse);
        std::mem::swap(self.borrow_vec_mut().deref_mut(), &mut elements);
        res?;

        if !elements.is_empty() {
            return Err(vm.new_value_error("list modified during sort"));
        }

        Ok(())
    }

    #[pyclassmethod]
    fn __class_getitem__(cls: PyTypeRef, args: PyObjectRef, vm: &VirtualMachine) -> PyGenericAlias {
        PyGenericAlias::from_args(cls, args, vm)
    }
}

fn extract_cloned<F, R>(obj: &PyObject, mut f: F, vm: &VirtualMachine) -> PyResult<Vec<R>>
where
    F: FnMut(PyObjectRef) -> PyResult<R>,
{
    use crate::builtins::PyTuple;
    if let Some(tuple) = obj.downcast_ref_if_exact::<PyTuple>(vm) {
        tuple.iter().map(|x| f(x.clone())).collect()
    } else if let Some(list) = obj.downcast_ref_if_exact::<PyList>(vm) {
        list.borrow_vec().iter().map(|x| f(x.clone())).collect()
    } else {
        let iter = obj.to_owned().get_iter(vm)?;
        let iter = iter.iter::<PyObjectRef>(vm)?;
        let len = obj.to_sequence().length_opt(vm).transpose()?.unwrap_or(0);
        let mut v = Vec::with_capacity(len);
        for x in iter {
            v.push(f(x?)?);
        }
        v.shrink_to_fit();
        Ok(v)
    }
}

impl MutObjectSequenceOp for PyList {
    type Inner = [PyObjectRef];

    fn do_get(index: usize, inner: &[PyObjectRef]) -> Option<&PyObjectRef> {
        inner.get(index)
    }

    fn do_lock(&self) -> impl std::ops::Deref<Target = [PyObjectRef]> {
        self.borrow_vec()
    }
}

impl Constructor for PyList {
    type Args = FuncArgs;

    fn py_new(cls: PyTypeRef, _args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        Self::default().into_ref_with_type(vm, cls).map(Into::into)
    }
}

impl Initializer for PyList {
    type Args = OptionalArg<PyObjectRef>;

    fn init(zelf: PyRef<Self>, iterable: Self::Args, vm: &VirtualMachine) -> PyResult<()> {
        let mut elements = if let OptionalArg::Present(iterable) = iterable {
            iterable.try_to_value(vm)?
        } else {
            vec![]
        };
        std::mem::swap(zelf.borrow_vec_mut().deref_mut(), &mut elements);
        Ok(())
    }
}

impl AsMapping for PyList {
    fn as_mapping() -> &'static PyMappingMethods {
        static AS_MAPPING: PyMappingMethods = PyMappingMethods {
            length: atomic_func!(|mapping, _vm| Ok(PyList::mapping_downcast(mapping).__len__())),
            subscript: atomic_func!(
                |mapping, needle, vm| PyList::mapping_downcast(mapping)._getitem(needle, vm)
            ),
            ass_subscript: atomic_func!(|mapping, needle, value, vm| {
                let zelf = PyList::mapping_downcast(mapping);
                if let Some(value) = value {
                    zelf._setitem(needle, value, vm)
                } else {
                    zelf._delitem(needle, vm)
                }
            }),
        };
        &AS_MAPPING
    }
}

impl AsSequence for PyList {
    fn as_sequence() -> &'static PySequenceMethods {
        static AS_SEQUENCE: PySequenceMethods = PySequenceMethods {
            length: atomic_func!(|seq, _vm| Ok(PyList::sequence_downcast(seq).__len__())),
            concat: atomic_func!(|seq, other, vm| {
                PyList::sequence_downcast(seq)
                    .concat(other, vm)
                    .map(|x| x.into())
            }),
            repeat: atomic_func!(|seq, n, vm| {
                PyList::sequence_downcast(seq)
                    .repeat(n, vm)
                    .map(|x| x.into())
            }),
            item: atomic_func!(|seq, i, vm| {
                PyList::sequence_downcast(seq)
                    .borrow_vec()
                    .getitem_by_index(vm, i)
            }),
            ass_item: atomic_func!(|seq, i, value, vm| {
                let zelf = PyList::sequence_downcast(seq);
                if let Some(value) = value {
                    zelf.borrow_vec_mut().setitem_by_index(vm, i, value)
                } else {
                    zelf.borrow_vec_mut().delitem_by_index(vm, i)
                }
            }),
            contains: atomic_func!(|seq, target, vm| {
                let zelf = PyList::sequence_downcast(seq);
                zelf.mut_contains(vm, target)
            }),
            inplace_concat: atomic_func!(|seq, other, vm| {
                let zelf = PyList::sequence_downcast(seq);
                PyList::inplace_concat(zelf, other, vm)
            }),
            inplace_repeat: atomic_func!(|seq, n, vm| {
                let zelf = PyList::sequence_downcast(seq);
                Ok(PyList::irepeat(zelf.to_owned(), n, vm)?.into())
            }),
        };
        &AS_SEQUENCE
    }
}

impl Iterable for PyList {
    fn iter(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult {
        Ok(PyListIterator {
            internal: PyMutex::new(PositionIterInternal::new(zelf, 0)),
        }
        .into_pyobject(vm))
    }
}

impl Comparable for PyList {
    fn cmp(
        zelf: &Py<Self>,
        other: &PyObject,
        op: PyComparisonOp,
        vm: &VirtualMachine,
    ) -> PyResult<PyComparisonValue> {
        if let Some(res) = op.identical_optimization(zelf, other) {
            return Ok(res.into());
        }
        let other = class_or_notimplemented!(Self, other);
        let a = &*zelf.borrow_vec();
        let b = &*other.borrow_vec();
        a.iter()
            .richcompare(b.iter(), op, vm)
            .map(PyComparisonValue::Implemented)
    }
}

impl Representable for PyList {
    #[inline]
    fn repr_str(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<String> {
        let s = if zelf.__len__() == 0 {
            "[]".to_owned()
        } else if let Some(_guard) = ReprGuard::enter(vm, zelf.as_object()) {
            collection_repr(None, "[", "]", zelf.borrow_vec().iter(), vm)?
        } else {
            "[...]".to_owned()
        };
        Ok(s)
    }
}

fn do_sort(
    vm: &VirtualMachine,
    values: &mut Vec<PyObjectRef>,
    key_func: Option<PyObjectRef>,
    reverse: bool,
) -> PyResult<()> {
    let op = if reverse {
        PyComparisonOp::Lt
    } else {
        PyComparisonOp::Gt
    };
    let cmp = |a: &PyObjectRef, b: &PyObjectRef| a.rich_compare_bool(b, op, vm);

    if let Some(ref key_func) = key_func {
        let mut items = values
            .iter()
            .map(|x| Ok((x.clone(), key_func.call((x.clone(),), vm)?)))
            .collect::<Result<Vec<_>, _>>()?;
        timsort::try_sort_by_gt(&mut items, |a, b| cmp(&a.1, &b.1))?;
        *values = items.into_iter().map(|(val, _)| val).collect();
    } else {
        timsort::try_sort_by_gt(values, cmp)?;
    }

    Ok(())
}

#[pyclass(module = false, name = "list_iterator", traverse)]
#[derive(Debug)]
pub struct PyListIterator {
    internal: PyMutex<PositionIterInternal<PyListRef>>,
}

impl PyPayload for PyListIterator {
    #[inline]
    fn class(ctx: &Context) -> &'static Py<PyType> {
        ctx.types.list_iterator_type
    }
}

#[pyclass(with(Unconstructible, IterNext, Iterable))]
impl PyListIterator {
    #[pymethod]
    fn __length_hint__(&self) -> usize {
        self.internal.lock().length_hint(|obj| obj.__len__())
    }

    #[pymethod]
    fn __setstate__(&self, state: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        self.internal
            .lock()
            .set_state(state, |obj, pos| pos.min(obj.__len__()), vm)
    }

    #[pymethod]
    fn __reduce__(&self, vm: &VirtualMachine) -> PyTupleRef {
        self.internal
            .lock()
            .builtins_iter_reduce(|x| x.clone().into(), vm)
    }
}
impl Unconstructible for PyListIterator {}

impl SelfIter for PyListIterator {}
impl IterNext for PyListIterator {
    fn next(zelf: &Py<Self>, _vm: &VirtualMachine) -> PyResult<PyIterReturn> {
        zelf.internal.lock().next(|list, pos| {
            let vec = list.borrow_vec();
            Ok(PyIterReturn::from_result(vec.get(pos).cloned().ok_or(None)))
        })
    }
}

#[pyclass(module = false, name = "list_reverseiterator", traverse)]
#[derive(Debug)]
pub struct PyListReverseIterator {
    internal: PyMutex<PositionIterInternal<PyListRef>>,
}

impl PyPayload for PyListReverseIterator {
    #[inline]
    fn class(ctx: &Context) -> &'static Py<PyType> {
        ctx.types.list_reverseiterator_type
    }
}

#[pyclass(with(Unconstructible, IterNext, Iterable))]
impl PyListReverseIterator {
    #[pymethod]
    fn __length_hint__(&self) -> usize {
        self.internal.lock().rev_length_hint(|obj| obj.__len__())
    }

    #[pymethod]
    fn __setstate__(&self, state: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        self.internal
            .lock()
            .set_state(state, |obj, pos| pos.min(obj.__len__()), vm)
    }

    #[pymethod]
    fn __reduce__(&self, vm: &VirtualMachine) -> PyTupleRef {
        self.internal
            .lock()
            .builtins_reversed_reduce(|x| x.clone().into(), vm)
    }
}
impl Unconstructible for PyListReverseIterator {}

impl SelfIter for PyListReverseIterator {}
impl IterNext for PyListReverseIterator {
    fn next(zelf: &Py<Self>, _vm: &VirtualMachine) -> PyResult<PyIterReturn> {
        zelf.internal.lock().rev_next(|list, pos| {
            let vec = list.borrow_vec();
            Ok(PyIterReturn::from_result(vec.get(pos).cloned().ok_or(None)))
        })
    }
}

pub fn init(context: &Context) {
    let list_type = &context.types.list_type;
    PyList::extend_class(context, list_type);

    PyListIterator::extend_class(context, context.types.list_iterator_type);
    PyListReverseIterator::extend_class(context, context.types.list_reverseiterator_type);
}
