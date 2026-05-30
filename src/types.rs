#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Type {
    Int,
    Char,
    Pointer(Box<Type>),
    Array(Box<Type>, usize),
}

impl Type {
    pub fn size(&self) -> usize {
        match self {
            Type::Int => 4,
            Type::Char => 1,
            Type::Pointer(_) => 8,
            Type::Array(elem, n) => elem.size() * n,
        }
    }

    /// 数组退化为指向元素的指针；其余类型不变。
    pub fn decay(&self) -> Type {
        match self {
            Type::Array(elem, _) => Type::Pointer(elem.clone()),
            other => other.clone(),
        }
    }

    /// 指针或数组的被指/元素类型。
    pub fn pointee(&self) -> Option<&Type> {
        match self {
            Type::Pointer(t) | Type::Array(t, _) => Some(t),
            _ => None,
        }
    }

    pub fn is_pointer_like(&self) -> bool {
        matches!(self, Type::Pointer(_) | Type::Array(..))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes() {
        assert_eq!(Type::Int.size(), 4);
        assert_eq!(Type::Char.size(), 1);
        assert_eq!(Type::Pointer(Box::new(Type::Int)).size(), 8);
        assert_eq!(Type::Array(Box::new(Type::Int), 10).size(), 40);
    }

    #[test]
    fn decay_and_pointee() {
        let arr = Type::Array(Box::new(Type::Char), 5);
        assert_eq!(arr.decay(), Type::Pointer(Box::new(Type::Char)));
        assert_eq!(Type::Pointer(Box::new(Type::Int)).pointee(), Some(&Type::Int));
        assert_eq!(Type::Int.pointee(), None);
    }
}
