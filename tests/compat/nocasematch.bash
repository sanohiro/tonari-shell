# Compare this script's stdout and exit status under Bash and Tonari.
shopt -s nocasematch extglob
patterns=(
    a A '*A' 'a*' '*a*' 'A?' '?[B]' '[A-Z]' '[^B-Z]' '[[:upper:]]' '[[:lower:]]'
    '[aÉ]' '[a-cX-Z]' '@(a|É)' '+(a|B)' '!(A|b)' 'a?(B)' a/b 'a\*b' '\[a\]' '['
    'a b' '?' '*' 'Ä' 'é' '雪' '🌸'
)
for value in '' a A Z Ä ä é É 雪 🌸 AB ab Ångström a/b 'a*b' Ab-AB xy '[a]' 'a b'; do
    for pattern in "${patterns[@]}"; do
        case $value in
            $pattern) result=C ;;
            *) result=- ;;
        esac
        [[ $value == $pattern ]] && result+=+ || result+=-
        printf '%s\n' "$result ${value//$pattern/[&]}"
    done
done

for pattern in '?(Q)' '!(A|B)' '?(A)' '*' '@(|q)' Q '' '?'; do
    for value in '' A B bA Aab; do
        printf '[%s,%s]: <%s> <%s> <%s> <%s>\n' "$value" "$pattern" \
            "${value/$pattern/[&]}" "${value//$pattern/[&]}" \
            "${value/#$pattern/[&]}" "${value/%$pattern/[&]}"
    done
done
