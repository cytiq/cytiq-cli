pub fn greeting() -> &'static str {
    "Hello, world!"
}

#[cfg(test)]
mod tests {
    use super::greeting;

    #[test]
    fn returns_default_greeting() {
        assert_eq!(greeting(), "Hello, world!");
    }
}
