ARG BASE=debian:13-slim
FROM ${BASE}
RUN apt-get update && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
    python3 gcc libc6-dev make locales-all diffutils bsdextrautils && rm -rf /var/lib/apt/lists/*
# GNU glob2 asks locale -a for an explicit encoding suffix.
RUN test "$(LC_ALL=zh_TW locale charmap)" = BIG5 && test "$(LC_ALL=zh_HK locale charmap)" = BIG5-HKSCS \
    && ln -s zh_TW /usr/lib/locale/zh_TW.big5 && ln -s zh_HK /usr/lib/locale/zh_HK.big5hkscs
ARG GNU_SHA256
COPY bash-src.tar.gz /tmp/bash-src.tar.gz
RUN echo "${GNU_SHA256}  /tmp/bash-src.tar.gz" | sha256sum -c - && mkdir /gnu /helpers \
    && tar -xzf /tmp/bash-src.tar.gz --strip-components=1 -C /gnu && rm /tmp/bash-src.tar.gz \
    && cd /helpers && /gnu/configure --without-bash-malloc > /dev/null \
    && make version.h > /dev/null \
    && for helper in recho zecho printenv xcase; do \
        gcc -std=gnu17 -include string.h -DHAVE_CONFIG_H -I. -I/gnu -I/gnu/include \
            -o "$helper" "/gnu/support/$helper.c" || exit 1; \
    done
USER 65534:65534
WORKDIR /repo
ENTRYPOINT ["sh", "/repo/tests/compat/linux.sh"]
