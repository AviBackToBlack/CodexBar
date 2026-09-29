use super::*;

fn site(raw: &str) -> Option<QoderSite> {
    site_for_manual_header(raw)
}

#[test]
fn manual_cookie_header_can_route_to_qoder_china_site() {
    assert_eq!(site("sid=abc"), Some(QoderSite::International));
    assert_eq!(
        site("sid=qoder.com.cn-looking-value"),
        Some(QoderSite::International)
    );
    assert_eq!(
        site("sid=abc; note=curl https://qoder.com.cn"),
        Some(QoderSite::International)
    );
    assert_eq!(
        site("sid=abc; redirect=https://example.com/curl"),
        Some(QoderSite::International)
    );
    assert_eq!(
        site("sid=abc; Domain=.qoder.com.cn"),
        Some(QoderSite::China)
    );
    assert_eq!(site("sid=abc; Domain=qoder.com.cn"), Some(QoderSite::China));
    assert_eq!(
        site("sid=abc; Domain=www.qoder.com.cn"),
        Some(QoderSite::China)
    );
    assert_eq!(
        site("curl https://qoder.com.cn -H 'Cookie: sid=abc'"),
        Some(QoderSite::China)
    );
    assert_eq!(
        site("HTTPS_PROXY=http://127.0.0.1:8080 curl https://qoder.com.cn"),
        Some(QoderSite::China)
    );
    assert_eq!(
        site(
            "HTTPS_PROXY=http://127.0.0.1:8080 \\\ncurl https://qoder.com.cn -H 'Cookie: sid=abc'"
        ),
        Some(QoderSite::China)
    );
    assert_eq!(
        site("\\\ncurl https://qoder.com.cn -H 'Cookie: sid=abc'"),
        Some(QoderSite::China)
    );
    assert_eq!(
        site("\\\r\ncurl https://qoder.com -H 'Cookie: sid=abc'"),
        Some(QoderSite::International)
    );
    assert_eq!(
        site(concat!(
            "curl https://qoder.com -H 'Origin: https://qoder.com' ",
            "-H 'Referer: https://qoder.com/account/usage' -H 'Cookie: sid=abc'"
        )),
        Some(QoderSite::International)
    );
    assert_eq!(
        site(concat!(
            "curl https://qoder.com.cn -H 'Origin: https://qoder.com.cn' ",
            "-H 'Referer: https://qoder.com.cn/account/usage' -H 'Cookie: sid=abc'"
        )),
        Some(QoderSite::China)
    );
    assert_eq!(
        site("curl https://www.qoder.com.cn -H 'Cookie: sid=abc'"),
        Some(QoderSite::China)
    );
    assert_eq!(
        site("curl --url https://qoder.com.cn -H 'Cookie: sid=abc'"),
        Some(QoderSite::China)
    );
    assert_eq!(
        site("curl --url https://qoder.com --data 'x=1; Domain=qoder.com.cn'"),
        Some(QoderSite::International)
    );
    assert_eq!(
        site("curl https://qoder.com --data 'GET /account/usage HTTP/1.1\nHost: qoder.com.cn'"),
        None
    );
    assert_eq!(
        site("GET https://qoder.com.cn/account/usage"),
        Some(QoderSite::China)
    );
    assert_eq!(
        site("GET /account/usage HTTP/1.1\nHost: qoder.com.cn"),
        Some(QoderSite::China)
    );
    assert_eq!(
        site("GET /account/usage HTTP/1.1\nHost: www.qoder.com.cn"),
        Some(QoderSite::China)
    );
    assert_eq!(
        site("GET /account/usage HTTP/1.1\nHost: qoder.com.cn:443"),
        Some(QoderSite::China)
    );
    assert_eq!(
        site("GET /account/usage HTTP/1.1\nHost: qoder.com.cn:evil"),
        None
    );
    assert_eq!(
        site("GET /account/usage HTTP/1.1\nHost: qoder.com.cn:"),
        None
    );
    assert_eq!(
        site("GET /account/usage HTTP/1.1\nHost: qoder.com.cn:65536"),
        None
    );
    assert_eq!(
        site("GET /account/usage HTTP/1.1\nHost: qoder.com.cn:443:444"),
        None
    );
    assert_eq!(
        site("GET /account/usage HTTP/1.1\nHost: qoder.com"),
        Some(QoderSite::International)
    );
    assert_eq!(
        site("TRACE /account/usage HTTP/1.1\nHost: qoder.com.cn"),
        None
    );
    assert_eq!(site("CONNECT qoder.com.cn:443 HTTP/1.1"), None);
    assert_eq!(
        site("BREW /account/usage HTTP/1.1\nHost: qoder.com.cn"),
        None
    );
    assert_eq!(
        site(
            "curl -H 'Referer: https://qoder.com.cn/account/usage' https://qoder.com -H 'Cookie: sid=abc'"
        ),
        None
    );
    assert_eq!(
        site(
            "curl --proxy-header 'X: https://qoder.com.cn' https://qoder.com -H 'Cookie: sid=abc'"
        ),
        None
    );
    assert_eq!(site("curl -X GET https://qoder.com.cn"), None);
    assert_eq!(site("sudo curl https://qoder.com.cn"), None);
    assert_eq!(site("sid=abc; curl https://qoder.com.cn"), None);
    assert_eq!(
        site("curl https://qoder.com/account https://qoder.com/profile"),
        None
    );
    assert_eq!(
        site("curl --url https://qoder.com/account https://qoder.com/profile"),
        None
    );
    assert_eq!(
        site("curl https://qoder.com https://qoder.com.cn -H 'Cookie: sid=abc'"),
        None
    );
    assert_eq!(site("curl https://example.com -H 'Cookie: sid=abc'"), None);
    assert_eq!(
        site("GET https://qoder.com/account/usage HTTP/1.1\nHost: qoder.com.cn"),
        None
    );
    assert_eq!(
        site("GET https://qoder.com/account/usage HTTP/1.1\nHost: example.com"),
        None
    );
    assert_eq!(site("GET /account/usage HTTP/1.1\nHost: example.com"), None);
}

#[test]
fn manual_curl_host_headers_must_match_authoritative_qoder_target() {
    assert_eq!(
        site("curl https://qoder.com -H 'Host: qoder.com' -H 'Cookie: sid=abc'"),
        Some(QoderSite::International)
    );
    assert_eq!(
        site("curl https://qoder.com.cn -H 'Host: www.qoder.com.cn:443' -H 'Cookie: sid=abc'"),
        Some(QoderSite::China)
    );
    assert_eq!(
        site("curl https://qoder.com.cn -sH 'Host: qoder.com.cn' -H 'Cookie: sid=abc'"),
        Some(QoderSite::China)
    );
    assert_eq!(
        site("curl https://qoder.com.cn -fsSLHHost:qoder.com.cn -H 'Cookie: sid=abc'"),
        Some(QoderSite::China)
    );
    assert_eq!(
        site("curl https://qoder.com.cn -HHost:qoder.com.cn -H 'Cookie: sid=abc'"),
        Some(QoderSite::China)
    );
    assert_eq!(
        site("curl https://qoder.com.cn --header=Host:qoder.com.cn -H 'Cookie: sid=abc'"),
        Some(QoderSite::China)
    );
    assert_eq!(
        site("curl https://qoder.com.cn \\\n-H 'Host: qoder.com.cn' \\\r\n-H 'Cookie: sid=abc'"),
        Some(QoderSite::China)
    );
    assert_eq!(
        site("curl https://qoder.com -H 'Host: qoder.com.cn' -H 'Cookie: sid=abc'"),
        None
    );
    assert_eq!(
        site("curl https://qoder.com.cn -H 'Host: qoder.com' -H 'Cookie: sid=abc'"),
        None
    );
    assert_eq!(
        site("curl https://qoder.com.cn -H 'Host: qoder.com.cn:evil' -H 'Cookie: sid=abc'"),
        None
    );
    assert_eq!(
        site(
            "curl https://qoder.com.cn -H 'Host: qoder.com.cn' -H 'Host: qoder.com' -H 'Cookie: sid=abc'"
        ),
        None
    );
    assert_eq!(
        site("curl https://qoder.com -sH 'Host: qoder.com.cn' -H 'Cookie: sid=abc'"),
        None
    );
    assert_eq!(
        site("curl https://qoder.com -fsSLHHost:qoder.com.cn -H 'Cookie: sid=abc'"),
        None
    );
    assert_eq!(
        site("curl https://qoder.com -HHost:qoder.com.cn -H 'Cookie: sid=abc'"),
        None
    );
    assert_eq!(
        site("curl https://qoder.com.cn -XH 'Host: qoder.com.cn' -H 'Cookie: sid=abc'"),
        None
    );
    assert_eq!(
        site("curl https://qoder.com -H @headers.txt -H 'Cookie: sid=abc'"),
        None
    );
    assert_eq!(
        site("curl https://qoder.com --header @- -H 'Cookie: sid=abc'"),
        None
    );
    assert_eq!(
        site("curl https://qoder.com.cn -H 'Host:' -H 'Cookie: sid=abc'"),
        None
    );
    assert_eq!(
        site("curl https://qoder.com.cn -H 'Host;' -H 'Cookie: sid=abc'"),
        None
    );
    assert_eq!(
        site("curl https://qoder.com.cn --header=Host\\; -H 'Cookie: sid=abc'"),
        None
    );
    assert_eq!(
        site("curl https://qoder.com.cn -sHHost\\; -H 'Cookie: sid=abc'"),
        None
    );
    assert_eq!(
        site("curl https://qoder.com.cn -K qoder.curlrc -H 'Cookie: sid=abc'"),
        None
    );
    assert_eq!(
        site("curl https://qoder.com.cn --config qoder.curlrc -H 'Cookie: sid=abc'"),
        None
    );
    assert_eq!(
        site("curl https://qoder.com.cn --config=qoder.curlrc -H 'Cookie: sid=abc'"),
        None
    );
    assert_eq!(
        site(concat!(
            "curl https://qoder.com --variable site=qoder.com.cn --expand-header 'Host: {{site}}' ",
            "-H 'Cookie: sid=abc'"
        )),
        None
    );
    assert_eq!(
        site(concat!(
            "curl https://qoder.com --variable site=qoder.com.cn --expand-url 'https://{{site}}' ",
            "-H 'Cookie: sid=abc'"
        )),
        None
    );
    assert_eq!(
        site("curl https://qoder.com.cn --expand-config '{{config}}' -H 'Cookie: sid=abc'"),
        None
    );
    assert_eq!(
        site("curl https://qoder.com.cn ; echo -H 'Cookie: sid=global'"),
        None
    );
    assert_eq!(
        site("curl https://qoder.com.cn | cat -H 'Cookie: sid=global'"),
        None
    );
    assert_eq!(
        site("curl https://qoder.com.cn > headers.txt -H 'Cookie: sid=abc'"),
        None
    );
    assert_eq!(
        site("curl https://qoder.com && echo done -H 'Cookie: sid=abc'"),
        None
    );
    assert_eq!(
        site("curl 'https://qoder.com/account/usage?a=1&b=2;next=ok' -H 'Cookie: sid=abc'"),
        Some(QoderSite::International)
    );
    assert_eq!(
        site("curl https://qoder.com -H 'X-Note: a;b|c&d=<e>' -H 'Cookie: sid=abc'"),
        Some(QoderSite::International)
    );
}

#[test]
fn manual_curl_rejects_shell_synthesis_and_injected_controls() {
    assert_eq!(
        site("curl https://qoder.com --location-trusted -H 'Cookie: sid=abc'"),
        None
    );
    assert_eq!(
        site("curl https://qoder.com -A $'agent\r\nHost: qoder.com.cn' -H 'Cookie: sid=abc'"),
        None
    );
    assert_eq!(
        site(concat!(
            "curl https://qoder.com --referer $'https://qoder.com\r\nHost: qoder.com.cn' ",
            "-H 'Cookie: sid=abc'"
        )),
        None
    );
    assert_eq!(
        site(
            "curl https://qoder.com -A \\'$'agent\\r\\nHost: qoder.com.cn'\\' -H 'Cookie: sid=abc'"
        ),
        None
    );
    assert_eq!(
        site("curl https://qoder.com -A \\'$'agent\r\nHost: qoder.com.cn'\\' -H 'Cookie: sid=abc'"),
        None
    );
    assert_eq!(
        site(
            "curl https://qoder.com -H 'User-Agent: agent\\\nHost: qoder.com.cn' -H 'Cookie: sid=abc'"
        ),
        None
    );
    assert_eq!(
        site("curl https://qoder.com -A $'agent\\r\\nHost: qoder.com.cn' -H 'Cookie: sid=abc'"),
        None
    );
    assert_eq!(
        site("curl https://qoder.com -A $(printf agent) -H 'Cookie: sid=abc'"),
        None
    );
    assert_eq!(
        site("curl https://qoder.com -A `printf agent` -H 'Cookie: sid=abc'"),
        None
    );
    assert_eq!(
        site("curl https://qoder.com -A $AGENT -H 'Cookie: sid=abc'"),
        None
    );
    assert_eq!(
        site("\"curl\" https://qoder.com.cn -A $AGENT -H 'Cookie: sid=abc'"),
        None
    );
    assert_eq!(
        site("'/usr/bin/curl' https://qoder.com.cn -A $AGENT -H 'Cookie: sid=abc'"),
        None
    );
    assert_eq!(
        site("\\curl https://qoder.com.cn -A $AGENT -H 'Cookie: sid=abc'"),
        None
    );
    assert_eq!(
        site("QODER_AGENT=$AGENT \\\ncurl https://qoder.com.cn -H 'Cookie: sid=abc'"),
        None
    );
    assert_eq!(
        site("curl https://qoder.com -H \"User-Agent: $AGENT\" -H 'Cookie: sid=abc'"),
        None
    );
    assert_eq!(
        site("curl https://qoder.com -A $\"agent\" -H 'Cookie: sid=abc'"),
        None
    );
    assert_eq!(
        site("curl https://qoder.com -H @<(printf 'Host: qoder.com.cn') -H 'Cookie: sid=abc'"),
        None
    );
    assert_eq!(
        site("curl https://qoder.com -A \\'literal\\' -H 'Cookie: sid=abc'"),
        Some(QoderSite::International)
    );
    assert_eq!(
        site("curl https://qoder.com -A \\\"literal\\\" -H 'Cookie: sid=abc'"),
        Some(QoderSite::International)
    );
    assert_eq!(
        site("curl https://qoder.com -A literal\\\\slash -H 'Cookie: sid=abc'"),
        Some(QoderSite::International)
    );
}

#[test]
fn manual_credential_binds_cookie_to_its_routed_site() {
    let china = manual_credential("curl https://qoder.com.cn -H 'Cookie: session=china'").unwrap();
    assert_eq!(china.site, QoderSite::China);
    assert_eq!(china.cookie_header, "session=china");

    let http =
        manual_credential("GET /account/usage HTTP/1.1\nHost: qoder.com.cn\nCookie: session=http")
            .unwrap();
    assert_eq!(http.site, QoderSite::China);
    assert_eq!(http.cookie_header, "session=http");

    let plain = manual_credential("Cookie: session=global").unwrap();
    assert_eq!(plain.site, QoderSite::International);
    assert_eq!(plain.cookie_header, "session=global");

    let lookalike = manual_credential("session=qoder.com.cn-looking-value").unwrap();
    assert_eq!(lookalike.site, QoderSite::International);
}

#[test]
fn manual_domain_attribute_routes_but_is_not_sent_as_a_cookie() {
    let credential = manual_credential("sid=abc; Domain=.qoder.com.cn").unwrap();
    assert_eq!(credential.site, QoderSite::China);
    assert_eq!(credential.cookie_header, "sid=abc");
}

#[test]
fn manual_credential_rejects_invalid_or_conflicting_routing() {
    for raw in [
        "curl https://qoder.com -H 'Host: qoder.com.cn' -H 'Cookie: sid=fixture'",
        "sid=abc; Domain=qoder.com; Domain=qoder.com.cn",
        "sid=abc; Domain=example.com",
        "curl https://example.com -H 'Cookie: sid=abc'",
        "curl https://qoder.com.cn",
        "",
    ] {
        assert_eq!(manual_credential(raw), None, "{raw}");
    }
}

#[test]
fn site_urls_stay_on_their_own_origin() {
    for site in QoderSite::ALL {
        assert!(site.usage_url().starts_with(site.origin()));
        assert_eq!(site.referer(), format!("{}/account/usage", site.origin()));
    }
    assert_eq!(QoderSite::International.domain(), "qoder.com");
    assert_eq!(QoderSite::China.domain(), "qoder.com.cn");
}
