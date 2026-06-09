use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Type {
    Void,
    Int,
    Char,
    Long,
    Double,
    Pointer(Box<Type>),
    Array(Box<Type>, usize),
    Struct(String),
    Union(String),
    /// 函数指针，携带返回类型（参数按位置传递，类型不在此记录）。
    FnPtr(Box<Type>),
}

impl Type {
    pub fn size(&self) -> usize {
        match self {
            Type::Void => 0,
            Type::Int => 4,
            Type::Char => 1,
            Type::Long => 8,
            Type::Double => 8,
            Type::Pointer(_) => 8,
            Type::FnPtr(_) => 8,
            Type::Array(elem, n) => elem.size() * n,
            Type::Struct(_) | Type::Union(_) => {
                unreachable!("use size_of with registry for aggregates")
            }
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Field {
    pub name: String,
    pub ty: Type,
    pub offset: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Aggregate {
    pub fields: Vec<Field>,
    pub size: usize,
    pub is_union: bool,
}

pub type Aggregates = HashMap<String, Aggregate>;

/// 函数签名（来自定义或原型声明）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Signature {
    pub ret: Type,
    /// 固定形参类型（不含可变参数部分）。
    pub params: Vec<Type>,
    pub fixed: usize,
    pub variadic: bool,
}

pub type Signatures = HashMap<String, Signature>;

/// 带聚合体注册表的大小计算。
pub fn size_of(ty: &Type, aggs: &Aggregates) -> usize {
    match ty {
        Type::Struct(name) | Type::Union(name) => aggs.get(name).map(|a| a.size).unwrap_or(0),
        Type::Array(elem, n) => size_of(elem, aggs) * n,
        other => other.size(),
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
    fn struct_layout_size() {
        let mut aggs: Aggregates = std::collections::HashMap::new();
        aggs.insert(
            "P".to_string(),
            Aggregate {
                fields: vec![
                    Field { name: "x".into(), ty: Type::Int, offset: 0 },
                    Field { name: "y".into(), ty: Type::Int, offset: 8 },
                ],
                size: 16,
                is_union: false,
            },
        );
        assert_eq!(size_of(&Type::Struct("P".into()), &aggs), 16);
        assert_eq!(
            size_of(&Type::Pointer(Box::new(Type::Struct("P".into()))), &aggs),
            8
        );
        assert_eq!(size_of(&Type::Int, &aggs), 4);
    }

    #[test]
    fn decay_and_pointee() {
        let arr = Type::Array(Box::new(Type::Char), 5);
        assert_eq!(arr.decay(), Type::Pointer(Box::new(Type::Char)));
        assert_eq!(Type::Pointer(Box::new(Type::Int)).pointee(), Some(&Type::Int));
        assert_eq!(Type::Int.pointee(), None);
    }
}
