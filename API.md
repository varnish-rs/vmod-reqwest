<!--

   !!!!!!  WARNING: DO NOT EDIT THIS FILE!

   This file was generated from the Varnish VMOD source code.
   It will be automatically updated on each build.

-->
# Varnish Module (VMOD) `reqwest`

```vcl
// Place import statement at the top of your VCL file
// This loads vmod from a standard location
import reqwest;

// Or load vmod from a specific file
import reqwest from "path/to/libreqwest.so";
```

### Constructor `reqwest.client([STRING base_url], [BOOL https], INT follow = 10, [DURATION timeout], [DURATION connect_timeout], BOOL auto_gzip = 1, BOOL auto_deflate = 1, BOOL auto_brotli = 1, BOOL accept_invalid_certs = 0, BOOL accept_invalid_hostnames = 0, [STRING http_proxy], [STRING https_proxy], [PROBE probe])`

Create a `client` object that can be used both for backend requests and in-vcl requests and will pool connections across them all. All arguments are optional.

`base_url` and `https`: dictates how the URL of a backend request is built:
- if `base_url` is specified, the full URL used is `base_url` + `bereq.url`, which means `base_url` nees to specify a scheme (e.g. `http://`) and a host (e.g. `www.example.com).
- otherwise, if `bereq.url`, doesn't start with a `/`, use it as-is
- otherwise, the URL is `http(s)://` + `bereq.http.host` + `bereq.url`, using `https` to decide on the scheme (will fail if there's no bereq.http.host)

`base_url` and `https` are mutually exclusive and can't be specified together.

* `[STRING base_url]`:
* `[BOOL https]`:
* `INT follow`:
`follow` dictates whether to follow redirects, and how many hops are allowed before becoming an error. A value of `0` or less will disable redirect folowing,
meaning you will actually receive 30X responses if they are sent by the server.
* `[DURATION timeout]`:
`connect_timeout` and `timeout` dictate how long we give a request to connect, and finish, respectively.
* `[DURATION connect_timeout]`:
* `BOOL auto_gzip`:
`auto_gzip`, `auto_deflate` and `auto_brotli`, if set, will automatically set the relevant `accept-encoding` values and automatically decompress the response
body. Note that this will only work if the `accept-encoding` header isn't already set AND if there's no `range` header. In practice, when contacting a backend, you will need to `unset bereq.http.accept-encoding;`, as Varnish sets it automatically.
* `BOOL auto_deflate`:
* `BOOL auto_brotli`:
* `BOOL accept_invalid_certs`:
avoid erroring on invalid certificates, for example self-signed ones. It's a dangerous option, use at your own risk!
* `BOOL accept_invalid_hostnames`:
even more dangerous, doesn't even require for the certificate hostname to match the server being contacted.
* `[STRING http_proxy]`:
HTTP proxy to send your requests through
* `[STRING https_proxy]`:
HTTPS proxy to send your requests through
* `[PROBE probe]`:
`probe` will work the same way as for regular backends, but there are a few details to be aware of:
- the health will only prevent a fetch for backends (i.e. when using `client.backend()`), not when creating free standing requests (`client.init()`/`client.send()`).
- if the `client` has a `base_url`, the probe will prepend it to its `.url` field to know which URL to probe.
- otherwise, it'll just use the `.url` field as-is (but will immediately error out if `.url` starts with a `/`).
- this means `client`s without`base_url` can actually probe a another server that the one used as a backend.

#### Method `VOID <object>.init(STRING name, STRING url, STRING method = "GET")`

reate an http request, identifying it by its `name`. The request is local to the VCL task it was created in. If a request already existed with the same name, it it simply dropped and replaced, i.e. it is NOT automatically sent.

* `STRING name`:
handle for the request, it'll be used by other methods to identify the transaction
* `STRING url`:
URL/path of the request
* `STRING method`:
HTTP method to use

#### Method `VOID <object>.send(STRING name)`

Actually send request `name`. This is non-blocking, and optional if you access the response. Any call to `status()`, `header()`, `body_as_string()` or `error()` will implicitly call `send()` if necessary and wait for the response to arrive.

`send()` is mainly useful in two cases:
- fire-and-forget: the response won't be checked, but you need the request to be sent away
- early send: you might want to send the request in `vcl_recv` but check the response in `vcl_deliver` to parallelize the VCL processing (backend fetch et al.) with the request.

* `STRING name`:
request handle

#### Method `VOID <object>.set_header(STRING name, STRING key, STRING value)`

Add a new header `name: value` to the unsent request named `name`. Calling this on a non-existing, or already sent request will trigger a VCL error.

* `STRING name`:
request handle
* `STRING key`:
header name
* `STRING value`:
header value

#### Method `VOID <object>.set_body(STRING name, STRING body)`

Set the body of the unsent request named `name`. As for `set_header()`, the request must exist and not have been sent.

* `STRING name`:
request handle
* `STRING body`:
the body to send

#### Method `VOID <object>.copy_headers_to_req(STRING name)`

Copy the native request headers (i.e. `req` or `bereq`) into the request named `name`.

* `STRING name`:
request handle

#### Method `INT <object>.status(STRING name)`

Retrieve the response status (send and wait if necessary), returns 0 if the reponse failed, but will cause a VCL errorif call on a non-existing request.

* `STRING name`:
request handle

#### Method `STRING <object>.header(STRING name, STRING key, [STRING sep])`

Retrieve the value of the first header named `key`, or returns NULL if it doesn't exist, or there was a transmission error.

* `STRING name`:
request handle
* `STRING key`:
header name
* `[STRING sep]`:
if set, concatenate all headers named `key`, using `sep` as separator

#### Method `STRING <object>.body_as_string(STRING name)`

Retrieve the response body, returns an empty string in case of error.

* `STRING name`:
request handle

#### Method `STRING <object>.error(STRING name)`

Returns the error string if request `name` failed.

* `STRING name`:
request handle

#### Method `BACKEND <object>.backend()`

Return a VCL backend built upon the `client` specification
